
			const audio = document.getElementById("stream");
			const ctx = new AudioContext();

			const conn = new RTCPeerConnection();
			const track = conn.addTransceiver("audio").receiver.track;
			conn.close();

			const stream = new MediaStreamAudioSourceNode(ctx, {
				mediaStream: new MediaStream([track])
			});



			let response = await fetch("/stream");
			let body = await response.body;

			const reader = body.getReader();

			let frames = new Uint8Array(384 * 128);
			let pos = 0;
			let buffers = [];

			let source = null;

			/**
				* @param {Uint8Array} frame 
			*/
			async function pushFrame(frame) {
				if (frame.length + pos < frames.length) {
					frames.set(frame, pos);
					pos += frame.length;

					if (frame.length + pos >= frames.length) {
						const buffer = await ctx.decodeAudioData(frames.buffer.slice(0, pos));

						buffers.push(buffer);

						if (buffers.length > 1 && !source) {
							function createSource(buffer) {
								source = new AudioBufferSourceNode(ctx);
								source.buffer = buffer;
								source.connect(ctx.destination);
								source.addEventListener("ended", () => {
									source = null;
									console.dir(buffers);
									if (buffers.length > 0) {
										createSource(buffers.shift());
									}
								});
								source.start(0);
							}

							createSource(buffers.shift());
						}

						pos = 0;
						frames.fill(0);
					}
				} else {
					throw new Error("TODO");
				}
			}


			let buffer = new Uint8Array();
			let need = 0;

			while (this.loop) {
				const { done, value } = await reader.read();
				if (done) break;

				let cursor = 0;


				while (cursor < value.length && this.loop) {
					if (need === 0) {
						if (value[cursor] === 0xFF && value[cursor + 1] & 0b11100000 === 0b11100000) {
							let header = value.slice(cursor, cursor + 4);
							// includes header
							let frame_size = frameSize(header);

							if (cursor + frame_size < value.length) {
								let frame = value.slice(cursor, cursor + frame_size);
								cursor += frame_size;
								await pushFrame(frame);
							} else {
								buffer = new Uint8Array(frame_size);
								buffer.set(value.slice(cursor));
								need = frame_size + cursor - value.length;
								cursor = value.length;
							}
						} else {
							break;
							let title_len = value[cursor] << 8 + value[cursor + 1];
							cursor += 2;
							let title = new TextDecoder().decode(value.slice(cursor, cursor + title_len));
							cursor += title_len;

							let artist_len = value[cursor] << 8 + value[cursor + 1];
							cursor += 2;
							let artist = new TextDecoder().decode(value.slice(cursor, cursor + artist_len));
							cursor += artist_len;

							await updateNowPlaying(title, artist);
							await updateQueue();
						}
					} else {
						if (cursor + need < value.length) {
							buffer.set(value.slice(cursor, cursor + need), buffer.length - need);
							cursor += need;

							need = 0;
							await pushFrame(buffer);
						} else {
							buffer.set(value.slice(cursor), buffer.length - need);
							need -= value.length - cursor;

							cursor = value.length;
						}
					}
				}
			}
		}