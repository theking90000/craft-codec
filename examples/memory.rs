//! Encode an object, restore its side metadata, and read a logical byte range.
use craft_codec::{Decoder, Encoder, Error, Framing, Identity, Metadata};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut encoder = Encoder::new(Framing::Fixed(8), Identity, Identity)?;
    let mut metadata = Metadata::new(encoder.config());
    let original = b"small independent frames";
    let mut object = Vec::new();
    let mut buffer = Vec::new();
    for (index, raw) in original.chunks(8).enumerate() {
        buffer.clear();
        buffer.extend_from_slice(raw);
        let sizes = encoder.encode_frame(index as u64, &mut buffer)?;
        object.extend_from_slice(&buffer);
        metadata.push(sizes)?;
    }
    // An application persists these parts separately, in trusted storage.
    let metadata = Metadata::from_parts(metadata.into_parts())?;
    let mut decoder = Decoder::new(metadata.config(), Identity, Identity)?;
    let (query, frames) = metadata.range(3, Some(19))?;
    let start = usize::try_from(query.start).map_err(|_| Error::Overflow)?;
    let end = usize::try_from(query.end).map_err(|_| Error::Overflow)?;
    let mut source = object
        .get(start..end)
        .ok_or(Error::InvalidStoredLength)?;
    let mut selected = Vec::new();
    for frame in frames {
        buffer.resize(frame.spec.stored_len(), 0);
        std::io::Read::read_exact(&mut source, &mut buffer)?;
        decoder.decode_frame(frame.spec, &mut buffer)?;
        selected.extend_from_slice(buffer.get(frame.selected).ok_or(Error::InvalidRange)?);
    }
    if original.get(3..19) != Some(selected.as_slice()) {
        return Err(Error::InvalidMetadata.into());
    }
    println!("{}", String::from_utf8(selected)?);
    Ok(())
}
