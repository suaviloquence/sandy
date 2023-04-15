use core::{
    future::Future,
    mem,
    pin::Pin,
    task::{Context, Poll, Waker},
};

use std::sync::{Arc, Mutex, RwLock};

type TailRef<T> = Arc<Mutex<Tail<T>>>;
// in future want to make this a std::sync::Weak ptr
type TailRefWeak<T> = Arc<Mutex<Tail<T>>>;
type NodeRef<T> = Arc<Node<T>>;

#[derive(Debug)]
struct Tail<T> {
    wakers: Vec<Waker>,
    next: Option<NodeRef<T>>,
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
    Next(NodeRef<T>),
    Tail(TailRefWeak<T>),
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
    tail: TailRef<T>,
}

#[derive(Debug)]
pub struct Receiver<T> {
    curr: Link<T>,
}

impl<T> Clone for Receiver<T> {
    fn clone(&self) -> Self {
        Self {
            curr: self.curr.clone(),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct SendError;

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SendError")
    }
}

impl std::error::Error for SendError {}

impl<T> Default for Sender<T> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Sender<T> {
    fn create_tail_node(n: usize) -> (TailRef<T>, Link<T>) {
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

    pub fn send(&mut self, msg: T) -> Result<(), SendError> {
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

#[derive(Debug)]
pub struct MsgRef<T> {
    inner: NodeRef<T>,
}

impl<T> Clone for  MsgRef<T> {
    fn clone(&self) -> Self {
        Self { inner: Arc::clone(&self.inner) }
    }
}

impl<T: PartialEq> PartialEq<T> for MsgRef<T> {
    fn eq(&self, other: &T) -> bool {
        &self.inner.msg == other
    }
}

impl<T> AsRef<T> for MsgRef<T> {
    fn as_ref(&self) -> &T {
        &self.inner.msg
    }
}

impl<T: PartialEq> PartialEq for MsgRef<T> {
    fn eq(&self, other: &Self) -> bool {
        self.inner.msg == other.inner.msg
    }
}

impl<T> core::ops::Deref for MsgRef<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.inner.msg
    }
}

#[derive(Debug)]
struct RecvFut<T> {
    tail: TailRefWeak<T>,
}

impl<T> Future for RecvFut<T> {
    type Output = Result<NodeRef<T>, RecvError>;

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

impl<T> Receiver<T> {
    fn try_recv_or_get_tail(
        &mut self,
    ) -> Result<Result<MsgRef<T>, TailRefWeak<T>>, RecvError> {
        match &self.curr {
            Link::Next(node) => {
                let next_guard = node.next.read().map_err(|_| RecvError)?;
                match &*next_guard {
                    Link::Next(node) => {
                        let node = Arc::clone(node);
                        drop(next_guard);
                        self.curr = Link::Next(Arc::clone(&node));
                        Ok(Ok(MsgRef { inner: node }))
                    }
                    Link::Tail(tail) => {
                        let tail_guard = tail.lock().map_err(|_| RecvError)?;
                        match &tail_guard.next {
                            Some(node) => {
                                let node = Arc::clone(node);
                                drop(tail_guard);
                                drop(next_guard);
                                self.curr = Link::Next(Arc::clone(&node));
                                Ok(Ok(MsgRef { inner: node }))
                            }
                            None => {
                                drop(tail_guard);
                                let tail = Arc::clone(tail);
                                drop(next_guard);
                                self.curr = Link::Tail(Arc::clone(&tail));
                                Ok(Err(tail))
                            }
                        }
                    }
                }
            }
            Link::Tail(t) => {
                let tail_guard = t.lock().map_err(|_| RecvError)?;
                match &tail_guard.next {
                    Some(node) => {
                        let node = Arc::clone(node);
                        drop(tail_guard);
                        self.curr = Link::Next(Arc::clone(&node));
                        Ok(Ok(MsgRef { inner: node }))
                    }
                    None => Ok(Err(Arc::clone(t))),
                }
            }
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


    impl<T: PartialEq> PartialEq for Node<T> {
        fn eq(&self, other: &Self) -> bool {
            self.msg == other.msg
        }
    }

    impl<T: PartialEq> PartialEq for Tail<T> {
        fn eq(&self, other: &Self) -> bool {
            self.next == other.next
        }
    }

    impl<T: PartialEq> PartialEq for Link<T> {
        fn eq(&self, other: &Self) -> bool {
            match (self, other) {
                (Link::Next(a), Link::Next(b)) => a == b,
                (Link::Tail(a), Link::Tail(b)) => Arc::ptr_eq(a, b),
                _ => false,
            }
        }
    }

    fn assert_ptr_eq<T: core::fmt::Debug>(a: &Arc<T>, b: &Arc<T>) {
        if !Arc::ptr_eq(a, b) {
            panic!("Assertion failed; a != b\n - a: `{:?}`\n - b: `{:?}`", a, b);
        }
    }

    // test of internals
    #[test]
    fn recv_cases() -> Result<(), Err> {
        // Next(a) -> Next(b): should return Ok(b) and update `curr` to b
        let next = Arc::new(Node {
            msg: 2i32,
            next: Default::default(),
        });
        let head = Arc::new(Node {
            msg: 1,
            next: RwLock::new(Link::Next(Arc::clone(&next))),
        });

        let mut rx = Receiver {
            curr: Link::Next(Arc::clone(&head)),
        };
        assert_eq!(rx.try_recv_or_get_tail()?.unwrap(), 2);
        assert_eq!(rx.curr, Link::Next(next));

        // Next(a) -> Tail(b) -> None: should return Err(b) and update `curr` to b
        let next = Arc::default();
        let head = Arc::new(Node {
            msg: 1i32,
            next: RwLock::new(Link::Tail(Arc::clone(&next))),
        });
        let mut rx = Receiver {
            curr: Link::Next(Arc::clone(&head)),
        };

        assert!(Arc::ptr_eq(&rx.try_recv_or_get_tail()?.unwrap_err(), &next));
        assert_eq!(rx.curr, Link::Tail(next));

        // Next(a) -> Tail(b) -> Some(Next(c)): should return Ok(c) and update curr to c
        let next = Arc::new(Node {
            msg: 2i32,
            next: Default::default(),
        });
        let head = Arc::new(Node {
            msg: 1i32,
            next: RwLock::new(Link::Tail(Arc::new(Mutex::new(Tail {
                next: Some(Arc::clone(&next)),
                ..Default::default()
            })))),
        });

        let mut rx = Receiver {
            curr: Link::Next(Arc::clone(&head)),
        };

        assert_eq!(rx.try_recv_or_get_tail()?.unwrap(), 2);
        assert_eq!(rx.curr, Link::Next(next));

        // Tail(a) -> None: should return Err(a) and `curr` should remain a
        let node = Arc::default();
        let mut rx: Receiver<()> = Receiver {
            curr: Link::Tail(Arc::clone(&node)),
        };
        assert_ptr_eq(&rx.try_recv_or_get_tail()?.unwrap_err(), &node);
        assert_eq!(rx.curr, Link::Tail(node));

        // Tail(a) -> Some(Node(b)): should return Ok(b) and `curr` should be b
        let next = Arc::new(Node {
            msg: 1,
            next: Default::default(),
        });

        let head = Arc::new(Mutex::new(Tail {
            next: Some(Arc::clone(&next)),
            ..Default::default()
        }));

        let mut rx = Receiver {
            curr: Link::Tail(Arc::clone(&head)),
        };
        assert_eq!(rx.try_recv_or_get_tail()?.unwrap(), 1);
        assert_eq!(rx.curr, Link::Next(next));

        Ok(())
    }

    #[tokio::test]
    async fn synchronized() -> Result<(), Err> {
        let mut sx = Sender::new();
        let mut rx = sx.subscribe();

        sx.send(1)?;
        assert_eq!(rx.recv().await?, 1);
        assert_eq!(rx.try_recv()?, None);
        sx.send(2)?;
        sx.send(3)?;
        assert_eq!(rx.recv().await?, 2);
        assert_eq!(rx.recv().await?, 3);
        assert_eq!(rx.try_recv()?, None);

        Ok(())
    }

    #[test]
    fn multiple_rx() -> Result<(), Err> {
        let mut sx = Sender::new();

        let mut r1 = sx.subscribe();
        let mut r2 = sx.subscribe();

        sx.send(1i32)?;

        let mut r3 = sx.subscribe();
        sx.send(2i32)?;

        assert_eq!(r1.try_recv()?.unwrap(), 1);
        assert_eq!(r3.try_recv()?.unwrap(), 2);

        assert_eq!(r1.try_recv()?.unwrap(), 2);
        assert_eq!(r1.try_recv()?, None);
        assert_eq!(r3.try_recv()?, None);

        assert_eq!(r2.try_recv()?.unwrap(), 1);
        assert_eq!(r2.try_recv()?.unwrap(), 2);
        assert_eq!(r2.try_recv()?, None);

        Ok(())
    }

    #[tokio::test]
    async fn basic_functionality() -> Result<(), Err> {
        let mut sx = Sender::new();
        let mut rx = sx.subscribe();

        let f1 = async move {
            sx.send(2)?;
            sx.send(3)?;
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            sx.send(4)?;

            Ok::<_, Err>(())
        };

        let f2 = async move {
            assert_eq!(rx.recv().await?, 2);
            assert_eq!(rx.recv().await?, 3);
            assert_eq!(rx.recv().await?, 4);
            assert_eq!(rx.try_recv()?, None);

            Ok::<_, Err>(())
        };

        let (r1, r2) = tokio::join!(f1, f2);

        r1?;
        r2?;
        Ok(())
    }
}
