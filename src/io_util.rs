use std::{
    fs::File,
    io::{BufReader, BufWriter, Read, Write},
    path::Path,
};

use anyhow::Context;

/// Opens the file and pipes it through a decompressor if the file extension indicates that it is compressed.
pub fn read_optionally_compressed_file<T>(
    path: impl AsRef<Path>,
    reader: impl FnOnce(&mut BufReader<dyn Read>) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let mut buf_reader = BufReader::new(open_optionally_compressed_file(path)?);
    reader(&mut buf_reader)
}

pub fn open_optionally_compressed_file(path: impl AsRef<Path>) -> anyhow::Result<Box<dyn Read>> {
    let path = path.as_ref();
    let extension = path.extension().and_then(|s| s.to_str());
    let file = File::open(path).with_context(|| format!("Failed to open file {:?}", path))?;

    if extension == Some("gz") || extension == Some("gzip") {
        let decoder = flate2::read::GzDecoder::new(file);
        Ok(Box::new(decoder))
    } else {
        Ok(Box::new(file))
    }
}

pub fn write_optionally_compressed_file<T>(
    path: impl AsRef<Path>,
    writer: impl FnOnce(&mut BufWriter<dyn Write>) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let path = path.as_ref();
    let extension = path.extension().and_then(|s| s.to_str());
    let file = File::create(path).with_context(|| format!("Failed to create file {:?}", path))?;

    if extension == Some("gz") || extension == Some("gzip") {
        let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut buf_writer = BufWriter::new(encoder);
        writer(&mut buf_writer)
    } else {
        writer(&mut BufWriter::new(file))
    }
}
