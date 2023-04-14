use core::{
    future::Future,
    mem,
    pin::Pin,
    task::{Context, Poll, Waker},
    fmt::Debug,
};

use std::sync::{Arc, Mutex, RwLock};

#[derive(Debug)]
struct Tail<T> {
    wakers: Vec<Waker>,
    next: Option<Arc<Node<T>>>,
}

impl<T> Default for Tail<T> {
    fn default() -> Self {
        Self {
            wakers: Default::default(),
            next: None,
        }
    }
}

#[derive(Debug)]
enum Link<T> {
    Next(Arc<Node<T>>),
    Tail(Arc<Mutex<Tail<T>>>),
}

impl<T> Default for Link<T> {
    fn default() -> Self {
        Self::Tail(Default::default())
    }
}

impl<T> Clone for Link<T> {
    fn clone(&self) -> Self {
        match self {
            // much more likely to be true
            Self::Next(next) => Self::Next(Arc::clone(next)),
            // cold branch
            Self::Tail(tail) => Self::Tail(Arc::clone(tail)),
        }
    }
}

#[derive(Debug)]
pub struct Node<T> {
    msg: T,
    next: RwLock<Link<T>>,
}

#[derive(Debug)]
pub struct Sender<T> {
    curr: Link<T>,
    /// number of wakers expected to wait on the next message
    n: usize,
    tail: Arc<Mutex<Tail<T>>>,
}

#[derive(Debug)]
pub struct Receiver<T> {
    curr: Link<T>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct SendError;

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SendError")
    }
}

impl std::error::Error for SendError {}

// remove later
impl<T: core::fmt::Debug> Sender<T> {
    fn create_tail_node(n: usize) -> (Arc<Mutex<Tail<T>>>, Link<T>) {
        let tail = Arc::new(Mutex::new(Tail {
            wakers: Vec::with_capacity(n),
            next: None,
        }));
        let node = Link::Tail(Arc::clone(&tail));
        (tail, node)
    }

    pub fn new() -> Self {
        let (tail, curr) = Self::create_tail_node(0);
        Self { curr, tail, n: 0 }
    }

    pub fn subscribe(&self) -> Receiver<T> {
        Receiver {
            curr: self.curr.clone(),
        }
    }

    pub async fn send(&mut self, msg: T) -> Result<(), SendError> {
        // here, n will be two send()s behind, but we save a mutex lock().
        // we could do self.n = self.wakers.lock().len() here to change this tradeoff
        let (new_tail, next) = Self::create_tail_node(self.n);
        let node = Arc::new(Node {
            msg,
            next: RwLock::new(next),
        });

        // likely branch - if not true, we waste a copy of the arc (on the unlikely branch) - not really a big deal
        if let Link::Next(prev) = mem::replace(&mut self.curr, Link::Next(Arc::clone(&node))) {
            *prev.next.write().map_err(|_| SendError)? = Link::Next(Arc::clone(&node));
        }

        let wakers = {
            let mut tail = self.tail.lock().map_err(|_| SendError)?;
            tail.next = Some(node);
            dbg!(&*tail);
            mem::take(&mut tail.wakers)
        };

        self.tail = new_tail;

        self.n = wakers.len();
        for waker in wakers {
            waker.wake();
        }


        Ok(())
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct RecvError;

impl std::fmt::Display for RecvError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RecvError")
    }
}

impl std::error::Error for RecvError {}

#[derive(Debug, Clone)]
pub struct MsgRef<T> {
    inner: Arc<Node<T>>,
}

impl<T> core::ops::Deref for MsgRef<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.inner.msg
    }
}

#[derive(Debug)]
struct RecvFut<T> {
    tail: Arc<Mutex<Tail<T>>>,
}

// TODO: remove type bound
impl<T: Debug> Future for RecvFut<T> {
    type Output = Result<Arc<Node<T>>, RecvError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut guard = match self.tail.lock() {
            Ok(t) => t,
            Err(_) => return Poll::Ready(Err(RecvError)),
        };

        if let Some(node) = &guard.next {
            return Poll::Ready(Ok(Arc::clone(node)));
        }

        guard.wakers.push(cx.waker().clone());

        Poll::Pending
    }
}

// TODO: remove constraint
impl<T: Debug> Receiver<T> {
    fn try_recv_or_get_tail(
        &mut self,
    ) -> Result<Result<MsgRef<T>, Arc<Mutex<Tail<T>>>>, RecvError> {
        let next = match &self.curr {
            Link::Next(node) => node.next.read().map_err(|_| RecvError)?.clone(),
            Link::Tail(t) => {
                let tail_guard = t.lock().map_err(|_| RecvError)?;
                match &tail_guard.next {
                    Some(node) => Link::Next(Arc::clone(&node)),
                    None => return Ok(Err(Arc::clone(&t))),
                }
            }
        };
        
        self.curr = next.clone();
        
        match next {
            Link::Next(inner) => Ok(Ok(MsgRef { inner })),
            Link::Tail(t) => Ok(Err(t)),
        }
    }

    #[inline]
    pub fn try_recv(&mut self) -> Result<Option<MsgRef<T>>, RecvError> {
        self.try_recv_or_get_tail().map(Result::ok)
    }

    pub async fn recv(&mut self) -> Result<MsgRef<T>, RecvError> {
        match self.try_recv_or_get_tail() {
            Ok(Ok(msg_ref)) => Ok(msg_ref),
            Ok(Err(tail)) => {
                let node = RecvFut { tail }.await?;
                self.curr = node.next.read().map_err(|_| RecvError)?.clone();
                Ok(MsgRef { inner: node })
            }
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::*;
    type Err = Box<dyn std::error::Error>;
    
    impl<T> PartialEq for Tail<T> {
        fn eq(&self, other: &Self) -> bool {
            self as *const _ == other as *const _
        }
    }
    
    impl<T> PartialEq for Link<T> {
        fn eq(&self, other: &Self) -> bool {
            match (self, other) {
                (Link::Next(a), Link::Next(b)) => a == b,
                (Link::Tail())
            }
        }
    }

    impl<T> Link<T> {
        fn next(&self) -> &Arc<Node<T>> {
            match self {
                Link::Next(n) => n,
                Link::Tail(t) => unreachable!("not next")
            }
        }

        fn tail(&self) -> &Arc<Mutex<Tail<T>>> {
            match self {
                Link::Next(n) => unreachable!("not tail"),
                Link::Tail(t) => t
            }
        }
    }
    
    #[test]
    fn recv_cases() {
        // N -> N
        let head = Link::Next(Arc::new(Node {
            msg: 1,
            next: RwLock::new(Link::Next(Arc::new(Node {
                msg: 2,
                next: Default::default(),
            })))
        }));
        

        assert_eq!(*rx.try_recv_or_get_tail().unwrap().unwrap(), 2);
        assert_eq!(rx.curr, head.next().next.into_inner().unwrap());
    }

    #[tokio::test]
    async fn synchronized() -> Result<(), Err> {
        let mut sx = Sender::new();
        let mut rx = sx.subscribe();
        
        sx.send(1).await?;
        assert_eq!(rx.try_recv()?.as_deref().copied(), Some(1));

        Ok(())
    }

    #[ignore]
    #[tokio::test]
    async fn basic_functionality() -> Result<(), Err> {
        let mut sx = Sender::new();
        let mut rx = sx.subscribe();

        let f1 = async move {
            sx.send(2).await?;
            sx.send(3).await?;
            Ok::<_, Err>(())
        };

        let f2 = async move {
            assert_eq!(*rx.recv().await?, 2);
            assert_eq!(*rx.recv().await?, 3);

            Ok::<_, Err>(())
        };

        let (r1, r2) = tokio::join!(f1, f2);

        r1?;
        r2?;
        Ok(())
    }
}
