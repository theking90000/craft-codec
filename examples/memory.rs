//! Encode an object, restore its side metadata, and read a logical byte range.
use craft_codec::{Decoder, Encoder, Framing, Identity, Metadata};

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
    let mut source = &object[query.start as usize..query.end as usize];
    let mut selected = Vec::new();
    for frame in frames {
        buffer.resize(frame.spec.stored_len(), 0);
        std::io::Read::read_exact(&mut source, &mut buffer)?;
        decoder.decode_frame(frame.spec, &mut buffer)?;
        selected.extend_from_slice(&buffer[frame.selected]);
    }
    assert_eq!(selected, original[3..19]);
    println!("{}", String::from_utf8(selected)?);
    Ok(())
}
