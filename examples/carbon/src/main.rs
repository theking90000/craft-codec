//! External CARBON adapters; the CRAFT library has no async dependency.
use carbon_io::{
    FrameBudget, FrameWriter, ReadFile, ReadScheduler, SchedulerConfig, WriteFile, WriteScheduler,
};
use craft_codec::{Aes256Gcm, Decoder, Encoder, Error, FrameIter, FrameSizes, Framing, Lz4, Metadata};
use futures::{Stream, StreamExt, executor::block_on, stream};
use std::{
    future::{Ready, ready},
    ops::Range,
    pin::Pin,
    task::{Context, Poll},
};

type Result<T> = std::result::Result<T, Error>;

struct Object {
    bytes: Vec<u8>,
    metadata: Metadata,
}

struct Destination {
    key: [u8; 32],
}

impl WriteFile<Vec<u8>> for Destination {
    type Error = Error;
    type Output = Object;
    type Open = Ready<Result<Writer>>;
    type Writer = Writer;
    fn frame_capacity(&self) -> u32 {
        4
    }
    fn open(&self) -> Self::Open {
        ready(
            Encoder::new(Framing::Fixed(256), Lz4::new(), Aes256Gcm::new(&self.key)).map(
                |encoder| Writer {
                    object: Object {
                        bytes: Vec::new(),
                        metadata: Metadata::new(encoder.config()),
                    },
                    encoder,
                    buffer: Vec::new(),
                    pending: None,
                    written: 0,
                },
            ),
        )
    }
}

struct Writer {
    encoder: Encoder<Lz4, Aes256Gcm>,
    object: Object,
    buffer: Vec<u8>,
    pending: Option<FrameSizes>,
    written: usize,
}

impl FrameWriter<Vec<u8>> for Writer {
    type Error = Error;
    type Output = Object;

    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, frame: &Vec<u8>) -> Poll<Result<()>> {
        let this = self.get_mut();
        if this.pending.is_none() {
            println!(
                "writing frame {} of {} bytes",
                this.object.metadata.frame_count(),
                frame.len()
            );
            // Preserve CARBON's original for replay. Do this only on the first poll.
            this.buffer.clear();
            this.buffer.extend_from_slice(frame);
            let sizes = this
                .encoder
                .encode_frame(this.object.metadata.frame_count(), &mut this.buffer)?;
            this.pending = Some(sizes);
            this.written = 0;
        }
        // Simulate partial transport writes. A real backend registers this waker
        // and wakes on readiness; only this in-memory demonstration self-wakes.
        let end = (this.written + 7).min(this.buffer.len());
        this.object
            .bytes
            .extend_from_slice(&this.buffer[this.written..end]);
        this.written = end;
        if end != this.buffer.len() {
            cx.waker().wake_by_ref();
            return Poll::Pending;
        }
        println!(
            "frame {} of {} bytes written",
            this.object.metadata.frame_count(),
            this.buffer.len()
        );
        this.object.metadata.push(this.pending.take().unwrap())?;
        Poll::Ready(Ok(()))
    }

    fn poll_finalize(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<Object>> {
        let this = self.get_mut();
        let replacement = Object {
            bytes: Vec::new(),
            metadata: Metadata::new(this.encoder.config()),
        };
        Poll::Ready(Ok(std::mem::replace(&mut this.object, replacement)))
    }
}

struct View<'a> {
    object: &'a Object,
    key: &'a [u8; 32],
    selection: Range<u64>,
}

struct SelectedFrame {
    buffer: Vec<u8>,
    selected: Range<usize>,
}

impl<'a> ReadFile<SelectedFrame> for View<'a> {
    type Error = Error;
    type Reader = Reader<'a>;
    type Open = Ready<Result<Self::Reader>>;
    fn frame_count(&self) -> u32 {
        // The example constructs a known nonempty, valid range. A general adapter
        // validates and stores this count in its constructor, before scheduling.
        u32::try_from(
            self.object
                .metadata
                .range(self.selection.start, Some(self.selection.end))
                .unwrap()
                .1
                .remaining(),
        )
        .unwrap()
    }
    fn open(&self) -> Self::Open {
        ready((|| {
            let (query, frames) = self
                .object
                .metadata
                .range(self.selection.start, Some(self.selection.end))?;
            Ok(Reader {
                source: &self.object.bytes[query.start as usize..query.end as usize],
                frames,
                decoder: Decoder::new(
                    self.object.metadata.config(),
                    Lz4::new(),
                    Aes256Gcm::new(self.key),
                )?,
                stopped: false,
            })
        })())
    }
}

struct Reader<'a> {
    source: &'a [u8],
    frames: FrameIter<'a>,
    decoder: Decoder<Lz4, Aes256Gcm>,
    stopped: bool,
}

impl Stream for Reader<'_> {
    type Item = Result<SelectedFrame>;
    fn poll_next(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if this.stopped {
            return Poll::Ready(None);
        }
        let Some(frame) = this.frames.next() else {
            return Poll::Ready(None);
        };
        let len = frame.spec.stored_len();
        if this.source.len() < len {
            this.stopped = true;
            return Poll::Ready(Some(Err(Error::InvalidStoredLength)));
        }
        // Owned Stream items need their own storage. CRAFT imposes no pool policy.
        let mut buffer = this.source[..len].to_vec();
        this.source = &this.source[len..];
        if let Err(error) = this.decoder.decode_frame(frame.spec, &mut buffer) {
            this.stopped = true;
            return Poll::Ready(Some(Err(error)));
        }
        Poll::Ready(Some(Ok(SelectedFrame {
            buffer,
            selected: frame.selected,
        })))
    }
}

fn main() -> Result<()> {
    block_on(async {
        // Demonstration key only. Supply a new independent secret per real object.
        let key = [42; 32];
        let raw: Vec<u8> = (0..800).map(|i| (i % 13) as u8).collect();
        let frames: Vec<Vec<u8>> = raw.chunks(256).map(<[u8]>::to_vec).collect();
        let mut writes = WriteScheduler::new(
            stream::iter(frames),
            stream::iter([Destination { key }]),
            FrameBudget::new(8),
            SchedulerConfig::default(),
        );
        let object = writes
            .next()
            .await
            .unwrap()
            .map_err(|_| Error::InvalidMetadata)?;
        println!(
            "Objet metadata: {:?} | >bytes {}",
            object.metadata,
            object.bytes.len()
        );
        assert!(writes.next().await.is_none());
        let view = View {
            object: &object,
            key: &key,
            selection: 500..713,
        };
        let mut reads = ReadScheduler::new(
            stream::iter([view]),
            FrameBudget::new(8),
            SchedulerConfig::default(),
        );
        let mut actual = Vec::new();
        while let Some(frame) = reads.next().await {
            let frame = frame.map_err(|_| Error::InvalidMetadata)?;
            println!(
                "reading frame of {} bytes, selected {}..{}",
                frame.buffer.len(),
                frame.selected.start,
                frame.selected.end
            );
            actual.extend_from_slice(&frame.buffer[frame.selected]);
        }
        assert_eq!(actual, raw[500..713]);
        println!(
            "{} selected bytes from {} stored frames",
            actual.len(),
            object.metadata.frame_count()
        );
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_writes_preserve_input_and_do_not_reencode() {
        let key = [3; 32];
        let destination = Destination { key };
        let mut writer = block_on(destination.open()).unwrap();
        let frame = vec![7; 256];
        let mut cx = Context::from_waker(futures::task::noop_waker_ref());
        let mut pending = 0;
        loop {
            match Pin::new(&mut writer).poll_write(&mut cx, &frame) {
                Poll::Pending => pending += 1,
                Poll::Ready(result) => {
                    result.unwrap();
                    break;
                }
            }
        }
        assert!(pending > 0);
        assert_eq!(frame, vec![7; 256]);
        assert_eq!(writer.object.metadata.frame_count(), 1);
        assert_eq!(
            writer.object.bytes.len() as u64,
            writer.object.metadata.stored_len()
        );
        let mut decoder = Decoder::new(
            writer.object.metadata.config(),
            Lz4::new(),
            Aes256Gcm::new(&key),
        )
        .unwrap();
        let mut encoded = writer.object.bytes;
        decoder
            .decode_frame(writer.object.metadata.frame(0).unwrap(), &mut encoded)
            .unwrap();
        assert_eq!(encoded, frame);
    }

    #[test]
    fn an_abandoned_attempt_can_be_replayed_without_modifying_the_source() {
        let destination = Destination { key: [4; 32] };
        let frame = vec![8; 256];
        let mut cx = Context::from_waker(futures::task::noop_waker_ref());
        let mut first = block_on(destination.open()).unwrap();
        assert!(
            Pin::new(&mut first)
                .poll_write(&mut cx, &frame)
                .is_pending()
        );
        drop(first);
        let mut retry = block_on(destination.open()).unwrap();
        while Pin::new(&mut retry)
            .poll_write(&mut cx, &frame)
            .is_pending()
        {}
        assert_eq!(retry.object.metadata.frame_count(), 1);
        assert_eq!(frame, vec![8; 256]);
    }
}
