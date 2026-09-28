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
        let end = this.written.saturating_add(7).min(this.buffer.len());
        let bytes = this
            .buffer
            .get(this.written..end)
            .ok_or(Error::InvalidStoredLength)?;
        this.object
            .bytes
            .extend_from_slice(bytes);
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
        let sizes = this.pending.take().ok_or(Error::InvalidMetadata)?;
        this.object.metadata.push(sizes)?;
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
    frame_count: u32,
}

impl<'a> View<'a> {
    fn new(object: &'a Object, key: &'a [u8; 32], selection: Range<u64>) -> Result<Self> {
        let frame_count = object
            .metadata
            .range(selection.start, Some(selection.end))?
            .1
            .remaining();
        Ok(Self {
            object,
            key,
            selection,
            frame_count: u32::try_from(frame_count).map_err(|_| Error::Overflow)?,
        })
    }
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
        self.frame_count
    }
    fn open(&self) -> Self::Open {
        ready((|| {
            let (query, frames) = self
                .object
                .metadata
                .range(self.selection.start, Some(self.selection.end))?;
            let start = usize::try_from(query.start).map_err(|_| Error::Overflow)?;
            let end = usize::try_from(query.end).map_err(|_| Error::Overflow)?;
            Ok(Reader {
                source: self
                    .object
                    .bytes
                    .get(start..end)
                    .ok_or(Error::InvalidStoredLength)?,
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
        let Some((encoded, remaining)) = this.source.split_at_checked(len) else {
            this.stopped = true;
            return Poll::Ready(Some(Err(Error::InvalidStoredLength)));
        };
        let mut buffer = encoded.to_vec();
        this.source = remaining;
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
            .ok_or(Error::InvalidMetadata)?
            .map_err(|_| Error::InvalidMetadata)?;
        println!(
            "Objet metadata: {:?} | >bytes {}",
            object.metadata,
            object.bytes.len()
        );
        if writes.next().await.is_some() {
            return Err(Error::InvalidMetadata);
        }
        let view = View::new(&object, &key, 500..713)?;
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
            actual.extend_from_slice(
                frame
                    .buffer
                    .get(frame.selected)
                    .ok_or(Error::InvalidRange)?,
            );
        }
        if raw.get(500..713) != Some(actual.as_slice()) {
            return Err(Error::InvalidMetadata);
        }
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
