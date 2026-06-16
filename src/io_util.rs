use std::{
    fs::File,
    io::{BufReader, BufWriter, Read, Write},
    path::Path,
};

use anyhow::{Context, bail};

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

    if extension == Some("gz")
        || extension == Some("gzip")
        || extension == Some("bgzf")
        || extension == Some("bgz")
        || extension == Some("bgzip")
    {
        let decoder = flate2::read::MultiGzDecoder::new(file);
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
    } else if extension == Some("bgzf") || extension == Some("bgz") || extension == Some("bgzip") {
        bail!("Blocked gzip compression is not supported for output files");
    } else {
        writer(&mut BufWriter::new(file))
    }
}

pub fn write_human_readable_time(seconds: f64, mut writer: impl Write) -> std::io::Result<()> {
    if seconds < 1e-8 {
        write!(writer, "{:.2}ns", seconds * 1e9)
    } else if seconds < 1e-7 {
        write!(writer, "{:.1}ns", seconds * 1e9)
    } else if seconds < 1e-6 {
        write!(writer, "{:.0}ns", seconds * 1e9)
    } else if seconds < 1e-5 {
        write!(writer, "{:.2}μs", seconds * 1e6)
    } else if seconds < 1e-4 {
        write!(writer, "{:.1}μs", seconds * 1e6)
    } else if seconds < 1e-3 {
        write!(writer, "{:.0}μs", seconds * 1e6)
    } else if seconds < 1e-2 {
        write!(writer, "{:.2}ms", seconds * 1e3)
    } else if seconds < 1e-1 {
        write!(writer, "{:.1}ms", seconds * 1e3)
    } else if seconds < 1e0 {
        write!(writer, "{:.0}ms", seconds * 1e3)
    } else if seconds < 1e1 {
        write!(writer, "{:.2}s", seconds)
    } else if seconds < 60.0 {
        write!(writer, "{:.1}s", seconds)
    } else if seconds < 3600.0 {
        write!(
            writer,
            "{:.0}m {:.0}s",
            (seconds / 60.0).floor(),
            seconds % 60.0
        )
    } else if seconds < 86400.0 {
        write!(
            writer,
            "{:.0}h {:.0}m",
            (seconds / 3600.0).floor(),
            (seconds / 60.0).round() % 60.0,
        )
    } else if seconds < 86400.0 * 100.0 {
        write!(
            writer,
            "{:.0}d {:.0}h",
            (seconds / 86400.0).floor(),
            (seconds / 3600.0).round() % 24.0,
        )
    } else {
        write!(writer, "{:.0}d", seconds / 86400.0)
    }
}

pub fn write_human_readable_time_to_string(seconds: f64) -> String {
    let mut buf = Vec::new();
    write_human_readable_time(seconds, &mut buf).unwrap();
    String::from_utf8(buf).unwrap()
}
