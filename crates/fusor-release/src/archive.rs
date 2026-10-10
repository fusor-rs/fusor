//! The zip writer is hand-rolled: current `zip` crate releases are
//! pre-releases carrying encryption and several compression backends. This
//! writes the classic format with deflate entries, no zip64. Timestamps are
//! fixed so the same binaries produce the same archive.
use crate::Result;
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::Path,
};

/// 1980-01-01 00:00, the earliest MS-DOS time.
const DOS_EPOCH_TIME: u16 = 0;
const DOS_EPOCH_DATE: u16 = 0x0021;
const LOCAL_SIGNATURE: u32 = 0x0403_4b50;
const CENTRAL_SIGNATURE: u32 = 0x0201_4b50;
const END_SIGNATURE: u32 = 0x0605_4b50;
const ZIP_VERSION: u16 = 20;
const DEFLATE: u16 = 8;
const LOCAL_HEADER_LENGTH: usize = 30;
const CENTRAL_HEADER_LENGTH: usize = 46;
const END_HEADER_LENGTH: usize = 22;

pub(super) fn tar_gz(directory: &Path, name: &str, archive: &Path) -> Result<()> {
    let file = File::create(archive)?;
    let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
    let mut builder = tar::Builder::new(encoder);
    builder.mode(tar::HeaderMode::Deterministic);
    builder.append_dir_all(name, directory)?;
    builder.into_inner()?.finish()?;
    Ok(())
}

/// Unix modes are recorded, so an executable stays executable when unpacked
/// on a Unix host.
pub(super) fn zip(directory: &Path, name: &str, archive: &Path) -> Result<()> {
    let mut output = Vec::new();
    let mut central = Vec::new();
    let mut entries = 0u16;
    for file in sorted_files(directory)? {
        let entry_name = format!("{name}/{}", file.file_name().to_string_lossy());
        let contents = fs::read(file.path())?;
        let crc = crc32fast::hash(&contents);
        let deflated = deflate(&contents)?;
        let offset = u32::try_from(output.len()).map_err(|_| "archive exceeds 4 GB")?;

        let entry = Entry {
            name: &entry_name,
            crc,
            compressed: deflated.len(),
            uncompressed: contents.len(),
            local_offset: offset,
            executable: executable(&file.path())?,
        };
        write_local_header(&mut output, &entry)?;
        output.extend_from_slice(&deflated);
        write_central_entry(&mut central, &entry)?;
        entries += 1;
    }
    let central_offset = u32::try_from(output.len()).map_err(|_| "archive exceeds 4 GB")?;
    let central_size = u32::try_from(central.len()).map_err(|_| "archive exceeds 4 GB")?;
    output.extend_from_slice(&central);
    write_end_of_central_directory(&mut output, entries, central_size, central_offset)?;
    fs::write(archive, output)?;
    Ok(())
}

/// Inflates every entry and checks its CRC and length.
pub(super) fn verify_zip(archive: &Path, expected_entries: usize) -> Result<()> {
    let bytes = fs::read(archive)?;
    let end = bytes
        .len()
        .checked_sub(END_HEADER_LENGTH)
        .ok_or("truncated archive")?;
    if read_u32(&bytes, end)? != END_SIGNATURE {
        return Err("no end-of-central-directory record; the archive has a trailing comment or is truncated".into());
    }
    let entries = read_u16(&bytes[end..], 10)? as usize;
    if entries != expected_entries {
        return Err(format!("archive holds {entries} entries, expected {expected_entries}").into());
    }
    let mut offset = read_u32(&bytes[end..], 16)? as usize;
    for _ in 0..entries {
        let header = read_field::<CENTRAL_HEADER_LENGTH>(&bytes, offset)?;
        if read_u32(&header, 0)? != CENTRAL_SIGNATURE {
            return Err("corrupt central directory".into());
        }
        verify_entry(&bytes, &header)?;
        let variable_length = [28, 30, 32]
            .into_iter()
            .try_fold(0, |length, at| -> Result<usize> {
                Ok(length + read_u16(&header, at)? as usize)
            })?;
        offset = offset
            .checked_add(CENTRAL_HEADER_LENGTH + variable_length)
            .filter(|offset| *offset <= bytes.len())
            .ok_or("truncated central directory")?;
    }
    Ok(())
}

fn verify_entry(bytes: &[u8], header: &[u8]) -> Result {
    let crc = read_u32(header, 16)?;
    let compressed = read_u32(header, 20)? as usize;
    let uncompressed = read_u32(header, 24)? as usize;
    let local = read_u32(header, 42)? as usize;
    let local_header = read_field::<LOCAL_HEADER_LENGTH>(bytes, local)?;
    if read_u32(&local_header, 0)? != LOCAL_SIGNATURE {
        return Err("corrupt local header".into());
    }
    let name = read_u16(&local_header, 26)? as usize;
    let extra = read_u16(&local_header, 28)? as usize;
    let start = local
        .checked_add(LOCAL_HEADER_LENGTH + name + extra)
        .ok_or("invalid local offset")?;
    let contents = bytes
        .get(start..)
        .and_then(|bytes| bytes.get(..compressed))
        .ok_or("entry data runs past the end of the archive")?;
    let inflated = inflate(contents)?;
    if inflated.len() != uncompressed || crc32fast::hash(&inflated) != crc {
        return Err("an archive entry does not match its recorded checksum".into());
    }
    Ok(())
}

fn sorted_files(directory: &Path) -> Result<Vec<fs::DirEntry>> {
    let mut files: Vec<_> = fs::read_dir(directory)?.collect::<std::io::Result<_>>()?;
    files.retain(|file| file.path().is_file());
    files.sort_by_key(std::fs::DirEntry::file_name);
    Ok(files)
}

#[cfg(unix)]
fn executable(path: &Path) -> Result<bool> {
    use std::os::unix::fs::PermissionsExt;
    Ok(fs::metadata(path)?.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn executable(path: &Path) -> Result<bool> {
    Ok(path.extension().is_some_and(|extension| extension == "exe"))
}

fn deflate(contents: &[u8]) -> Result<Vec<u8>> {
    let mut encoder =
        flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(contents)?;
    Ok(encoder.finish()?)
}

fn inflate(contents: &[u8]) -> Result<Vec<u8>> {
    let mut decoder = flate2::read::DeflateDecoder::new(contents);
    let mut out = Vec::new();
    decoder.read_to_end(&mut out)?;
    Ok(out)
}

fn read_field<const SIZE: usize>(bytes: &[u8], at: usize) -> Result<[u8; SIZE]> {
    let field = bytes
        .get(at..)
        .and_then(|bytes| bytes.get(..SIZE))
        .ok_or("truncated archive")?;
    Ok(field
        .try_into()
        .expect("the checked archive field has exactly SIZE bytes"))
}

fn read_u16(bytes: &[u8], at: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(read_field(bytes, at)?))
}

fn read_u32(bytes: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(read_field(bytes, at)?))
}

struct Entry<'a> {
    name: &'a str,
    crc: u32,
    compressed: usize,
    uncompressed: usize,
    local_offset: u32,
    executable: bool,
}

fn write_local_header(output: &mut Vec<u8>, entry: &Entry<'_>) -> Result<()> {
    let Entry {
        name,
        crc,
        compressed,
        uncompressed,
        ..
    } = *entry;
    output.extend_from_slice(&LOCAL_SIGNATURE.to_le_bytes());
    output.extend_from_slice(&ZIP_VERSION.to_le_bytes()); // version needed
    output.extend_from_slice(&0u16.to_le_bytes()); // flags
    output.extend_from_slice(&DEFLATE.to_le_bytes()); // deflate
    output.extend_from_slice(&DOS_EPOCH_TIME.to_le_bytes());
    output.extend_from_slice(&DOS_EPOCH_DATE.to_le_bytes());
    output.extend_from_slice(&crc.to_le_bytes());
    output.extend_from_slice(&size(compressed)?.to_le_bytes());
    output.extend_from_slice(&size(uncompressed)?.to_le_bytes());
    output.extend_from_slice(&length(name)?.to_le_bytes());
    output.extend_from_slice(&0u16.to_le_bytes()); // extra field
    output.extend_from_slice(name.as_bytes());
    Ok(())
}

fn write_central_entry(central: &mut Vec<u8>, entry: &Entry<'_>) -> Result<()> {
    let Entry {
        name,
        crc,
        compressed,
        uncompressed,
        local_offset,
        executable,
    } = *entry;
    // Unix permissions live in the high 16 bits of the external attributes.
    let mode: u32 = if executable { 0o100755 } else { 0o100644 };
    central.extend_from_slice(&CENTRAL_SIGNATURE.to_le_bytes());
    central.extend_from_slice(&0x031Eu16.to_le_bytes()); // made by Unix, 3.0
    central.extend_from_slice(&ZIP_VERSION.to_le_bytes()); // version needed
    central.extend_from_slice(&0u16.to_le_bytes()); // flags
    central.extend_from_slice(&DEFLATE.to_le_bytes()); // deflate
    central.extend_from_slice(&DOS_EPOCH_TIME.to_le_bytes());
    central.extend_from_slice(&DOS_EPOCH_DATE.to_le_bytes());
    central.extend_from_slice(&crc.to_le_bytes());
    central.extend_from_slice(&size(compressed)?.to_le_bytes());
    central.extend_from_slice(&size(uncompressed)?.to_le_bytes());
    central.extend_from_slice(&length(name)?.to_le_bytes());
    central.extend_from_slice(&0u16.to_le_bytes()); // extra field
    central.extend_from_slice(&0u16.to_le_bytes()); // comment
    central.extend_from_slice(&0u16.to_le_bytes()); // disk number
    central.extend_from_slice(&0u16.to_le_bytes()); // internal attributes
    central.extend_from_slice(&(mode << 16).to_le_bytes());
    central.extend_from_slice(&local_offset.to_le_bytes());
    central.extend_from_slice(name.as_bytes());
    Ok(())
}

fn write_end_of_central_directory(
    output: &mut Vec<u8>,
    entries: u16,
    size: u32,
    offset: u32,
) -> Result<()> {
    output.extend_from_slice(&END_SIGNATURE.to_le_bytes());
    output.extend_from_slice(&0u16.to_le_bytes()); // this disk
    output.extend_from_slice(&0u16.to_le_bytes()); // disk with central directory
    output.extend_from_slice(&entries.to_le_bytes());
    output.extend_from_slice(&entries.to_le_bytes());
    output.extend_from_slice(&size.to_le_bytes());
    output.extend_from_slice(&offset.to_le_bytes());
    output.extend_from_slice(&0u16.to_le_bytes()); // comment
    Ok(())
}

fn size(value: usize) -> Result<u32> {
    u32::try_from(value).map_err(|_| "zip entries larger than 4 GB need zip64".into())
}

fn length(name: &str) -> Result<u16> {
    u16::try_from(name.len()).map_err(|_| "archive entry name is too long".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_local_offsets_return_errors_without_panicking() {
        let file =
            std::env::temp_dir().join(format!("fusor-invalid-archive-{}.zip", std::process::id()));
        let mut bytes = vec![0; 68];
        bytes[..4].copy_from_slice(&[0x50, 0x4b, 0x01, 0x02]);
        bytes[46..50].copy_from_slice(&[0x50, 0x4b, 0x05, 0x06]);
        bytes[56..58].copy_from_slice(&1u16.to_le_bytes());
        for offset in [60u32, u32::MAX] {
            bytes[42..46].copy_from_slice(&offset.to_le_bytes());
            fs::write(&file, &bytes).unwrap();
            assert_eq!(
                verify_zip(&file, 1).unwrap_err().to_string(),
                "truncated archive"
            );
        }
        fs::remove_file(file).unwrap();
    }

    #[test]
    fn a_written_zip_round_trips_and_is_reproducible() {
        let root = std::env::temp_dir().join(format!("fusor-archive-{}", std::process::id()));
        let stage = root.join("stage");
        fs::create_dir_all(&stage).unwrap();
        fs::write(stage.join("fusor"), b"binary contents".repeat(100)).unwrap();
        fs::write(stage.join("LICENSE"), b"license").unwrap();

        let first = root.join("first.zip");
        let second = root.join("second.zip");
        zip(&stage, "fusor-0.1.0-test", &first).unwrap();
        zip(&stage, "fusor-0.1.0-test", &second).unwrap();

        verify_zip(&first, 2).unwrap();
        assert_eq!(fs::read(&first).unwrap(), fs::read(&second).unwrap());
        fs::remove_dir_all(root).unwrap();
    }
}
