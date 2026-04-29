use super::*;
use anyhow::Result;
use std::io::Write;
use std::path::Path;

fn set_raw_tar_path(header: &mut tar::Header, raw_path: &[u8]) {
    assert!(raw_path.len() < 100, "test tar path must fit old header");
    let bytes = header.as_mut_bytes();
    bytes[0..100].fill(0);
    bytes[0..raw_path.len()].copy_from_slice(raw_path);
}

fn write_tar_gz(
    path: &Path,
    write_entries: impl FnOnce(&mut tar::Builder<flate2::write::GzEncoder<std::fs::File>>) -> Result<()>,
) -> Result<()> {
    let file = std::fs::File::create(path)?;
    let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
    let mut builder = tar::Builder::new(encoder);
    write_entries(&mut builder)?;
    let encoder = builder.into_inner()?;
    encoder.finish()?;
    Ok(())
}

fn patch_zip_entry_unix_mode(zip_path: &Path, entry_name: &str, mode: u32) {
    let mut bytes = std::fs::read(zip_path).expect("read zip for patching");
    let mut offset = 0usize;
    while offset + 46 <= bytes.len() {
        let relative = bytes[offset..]
            .windows(4)
            .position(|window| window == b"PK\x01\x02")
            .expect("central directory header");
        let start = offset + relative;
        assert!(
            start + 46 <= bytes.len(),
            "central directory header truncated"
        );
        let name_len = u16::from_le_bytes([bytes[start + 28], bytes[start + 29]]) as usize;
        let extra_len = u16::from_le_bytes([bytes[start + 30], bytes[start + 31]]) as usize;
        let comment_len = u16::from_le_bytes([bytes[start + 32], bytes[start + 33]]) as usize;
        let name_start = start + 46;
        let name_end = name_start + name_len;
        assert!(name_end <= bytes.len(), "central directory name truncated");
        if &bytes[name_start..name_end] == entry_name.as_bytes() {
            bytes[start + 5] = 3;
            bytes[start + 38..start + 42].copy_from_slice(&(mode << 16).to_le_bytes());
            std::fs::write(zip_path, bytes).expect("write patched zip");
            return;
        }
        offset = name_end + extra_len + comment_len;
    }
    panic!("zip entry not found: {entry_name}");
}

#[test]
fn archive_extraction_zip_rejects_parent_traversal() {
    let temp = tempfile::tempdir().expect("tempdir");
    let zip_path = temp.path().join("traversal.zip");
    let file = std::fs::File::create(&zip_path).expect("create zip");
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default().unix_permissions(0o644);
    zip.start_file("../escape.txt", options)
        .expect("start zip entry");
    zip.write_all(b"escape").expect("write zip entry");
    zip.finish().expect("finish zip");

    let out_dir = temp.path().join("out");
    let err = extract_zip_to_dir(&zip_path, &out_dir).expect_err("zip traversal should fail");
    assert!(
        err.to_string().contains("parent directory"),
        "unexpected error: {err:#}"
    );
    assert!(!temp.path().join("escape.txt").exists());
}

#[test]
fn archive_extraction_tar_gz_rejects_parent_traversal() {
    let temp = tempfile::tempdir().expect("tempdir");
    let tar_path = temp.path().join("traversal.tar.gz");
    write_tar_gz(&tar_path, |builder| {
        let data = b"escape";
        let mut header = tar::Header::new_gnu();
        header.set_mode(0o644);
        header.set_size(data.len() as u64);
        set_raw_tar_path(&mut header, b"../escape.txt");
        header.set_cksum();
        builder.append(&header, &data[..])?;
        Ok(())
    })
    .expect("write tar.gz");

    let out_dir = temp.path().join("out");
    let err = extract_tar_gz_to_dir(&tar_path, &out_dir).expect_err("tar traversal should fail");
    assert!(
        err.to_string().contains("parent directory"),
        "unexpected error: {err:#}"
    );
    assert!(!temp.path().join("escape.txt").exists());
}

#[test]
fn archive_extraction_tar_gz_rejects_symlink_escape() {
    let temp = tempfile::tempdir().expect("tempdir");
    let tar_path = temp.path().join("symlink-escape.tar.gz");
    write_tar_gz(&tar_path, |builder| {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_mode(0o777);
        header.set_size(0);
        header.set_path("bin/ctx")?;
        header.set_link_name("../../outside")?;
        header.set_cksum();
        builder.append(&header, std::io::empty())?;
        Ok(())
    })
    .expect("write tar.gz");

    let out_dir = temp.path().join("out");
    let err = extract_tar_gz_to_dir(&tar_path, &out_dir).expect_err("symlink escape should fail");
    assert!(
        err.to_string().contains("escapes extraction root"),
        "unexpected error: {err:#}"
    );
    assert!(!temp.path().join("outside").exists());
}

#[test]
fn archive_extraction_zip_rejects_symlink_escape() {
    let temp = tempfile::tempdir().expect("tempdir");
    let zip_path = temp.path().join("symlink-escape.zip");
    let file = std::fs::File::create(&zip_path).expect("create zip");
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default().unix_permissions(0o120777);
    zip.start_file("bin/ctx", options)
        .expect("start zip symlink");
    zip.write_all(b"../../outside").expect("write zip symlink");
    zip.finish().expect("finish zip");
    patch_zip_entry_unix_mode(&zip_path, "bin/ctx", 0o120777);

    let out_dir = temp.path().join("out");
    let err = extract_zip_to_dir(&zip_path, &out_dir).expect_err("symlink escape should fail");
    assert!(
        err.to_string().contains("escapes extraction root"),
        "unexpected error: {err:#}"
    );
    assert!(!temp.path().join("outside").exists());
}

#[test]
fn archive_extraction_tar_gz_allows_in_root_hardlinks() {
    let temp = tempfile::tempdir().expect("tempdir");
    let tar_path = temp.path().join("hardlink.tar.gz");
    write_tar_gz(&tar_path, |builder| {
        let data = b"linked";
        let mut file_header = tar::Header::new_gnu();
        file_header.set_mode(0o755);
        file_header.set_size(data.len() as u64);
        file_header.set_cksum();
        builder.append_data(&mut file_header, "bin/source", &data[..])?;

        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Link);
        header.set_mode(0o777);
        header.set_size(0);
        header.set_path("bin/ctx")?;
        header.set_link_name("bin/source")?;
        header.set_cksum();
        builder.append(&header, std::io::empty())?;
        Ok(())
    })
    .expect("write tar.gz");

    let out_dir = temp.path().join("out");
    extract_tar_gz_to_dir(&tar_path, &out_dir).expect("extract hardlink");
    assert_eq!(
        std::fs::read(out_dir.join("bin/ctx")).expect("read hardlink"),
        b"linked"
    );
}

#[test]
fn archive_extraction_tar_gz_rejects_escaping_hardlink_target() {
    let temp = tempfile::tempdir().expect("tempdir");
    let tar_path = temp.path().join("hardlink-escape.tar.gz");
    write_tar_gz(&tar_path, |builder| {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Link);
        header.set_mode(0o777);
        header.set_size(0);
        header.set_path("bin/ctx")?;
        header.set_link_name("../outside")?;
        header.set_cksum();
        builder.append(&header, std::io::empty())?;
        Ok(())
    })
    .expect("write tar.gz");

    let out_dir = temp.path().join("out");
    let err = extract_tar_gz_to_dir(&tar_path, &out_dir).expect_err("hardlink escape should fail");
    assert!(
        err.to_string().contains("parent directory"),
        "unexpected error: {err:#}"
    );
}

#[test]
fn archive_extraction_tar_gz_skips_global_pax_header_with_empty_path() {
    let temp = tempfile::tempdir().expect("tempdir");
    let tar_path = temp.path().join("pax-global.tar.gz");
    write_tar_gz(&tar_path, |builder| {
        let pax_data = b"10 comment=x\n";
        let mut pax_header = tar::Header::new_gnu();
        pax_header.set_entry_type(tar::EntryType::XGlobalHeader);
        pax_header.set_mode(0o644);
        pax_header.set_size(pax_data.len() as u64);
        set_raw_tar_path(&mut pax_header, b"");
        pax_header.set_cksum();
        builder.append(&pax_header, &pax_data[..])?;

        let data = b"ok";
        let mut file_header = tar::Header::new_gnu();
        file_header.set_mode(0o644);
        file_header.set_size(data.len() as u64);
        file_header.set_cksum();
        builder.append_data(&mut file_header, "bin/goose", &data[..])?;
        Ok(())
    })
    .expect("write tar.gz");

    let out_dir = temp.path().join("out");
    extract_tar_gz_to_dir(&tar_path, &out_dir).expect("extract pax global header");
    assert_eq!(
        std::fs::read(out_dir.join("bin/goose")).expect("read file"),
        b"ok"
    );
}

#[cfg(unix)]
#[test]
fn archive_extraction_tar_gz_allows_in_root_symlink() {
    let temp = tempfile::tempdir().expect("tempdir");
    let tar_path = temp.path().join("safe-symlink.tar.gz");
    write_tar_gz(&tar_path, |builder| {
        let mut dir_header = tar::Header::new_gnu();
        dir_header.set_entry_type(tar::EntryType::Directory);
        dir_header.set_mode(0o755);
        dir_header.set_size(0);
        dir_header.set_cksum();
        builder.append_data(&mut dir_header, "bin", std::io::empty())?;

        let mut symlink_header = tar::Header::new_gnu();
        symlink_header.set_entry_type(tar::EntryType::Symlink);
        symlink_header.set_mode(0o777);
        symlink_header.set_size(0);
        symlink_header.set_path("bin/npm")?;
        symlink_header.set_link_name("../lib/node_modules/npm/bin/npm-cli.js")?;
        symlink_header.set_cksum();
        builder.append(&symlink_header, std::io::empty())?;
        Ok(())
    })
    .expect("write tar.gz");

    let out_dir = temp.path().join("out");
    extract_tar_gz_to_dir(&tar_path, &out_dir).expect("extract safe symlink");
    let target = std::fs::read_link(out_dir.join("bin/npm")).expect("read symlink");
    assert_eq!(target, Path::new("../lib/node_modules/npm/bin/npm-cli.js"));
}

#[cfg(unix)]
#[test]
fn archive_extraction_tar_gz_allows_safe_symlink_ancestor() {
    let temp = tempfile::tempdir().expect("tempdir");
    let tar_path = temp.path().join("safe-symlink-ancestor.tar.gz");
    write_tar_gz(&tar_path, |builder| {
        let mut target_dir_header = tar::Header::new_gnu();
        target_dir_header.set_entry_type(tar::EntryType::Directory);
        target_dir_header.set_mode(0o755);
        target_dir_header.set_size(0);
        target_dir_header.set_cksum();
        builder.append_data(&mut target_dir_header, "terminfo/32", std::io::empty())?;

        let mut symlink_header = tar::Header::new_gnu();
        symlink_header.set_entry_type(tar::EntryType::Symlink);
        symlink_header.set_mode(0o777);
        symlink_header.set_size(0);
        symlink_header.set_path("terminfo/2")?;
        symlink_header.set_link_name("32")?;
        symlink_header.set_cksum();
        builder.append(&symlink_header, std::io::empty())?;

        let data = b"entry";
        let mut file_header = tar::Header::new_gnu();
        file_header.set_mode(0o644);
        file_header.set_size(data.len() as u64);
        file_header.set_cksum();
        builder.append_data(&mut file_header, "terminfo/2/2621a", &data[..])?;
        Ok(())
    })
    .expect("write tar.gz");

    let out_dir = temp.path().join("out");
    extract_tar_gz_to_dir(&tar_path, &out_dir).expect("extract safe symlink ancestor");
    assert_eq!(
        std::fs::read(out_dir.join("terminfo/32/2621a")).expect("read symlink target file"),
        b"entry"
    );
}

#[cfg(unix)]
#[test]
fn archive_extraction_tar_gz_rejects_existing_symlink_ancestor_escape() {
    let temp = tempfile::tempdir().expect("tempdir");
    let tar_path = temp.path().join("existing-symlink-ancestor.tar.gz");
    write_tar_gz(&tar_path, |builder| {
        let data = b"escape";
        let mut file_header = tar::Header::new_gnu();
        file_header.set_mode(0o644);
        file_header.set_size(data.len() as u64);
        file_header.set_cksum();
        builder.append_data(&mut file_header, "link/file", &data[..])?;
        Ok(())
    })
    .expect("write tar.gz");

    let out_dir = temp.path().join("out");
    let outside = temp.path().join("outside");
    std::fs::create_dir_all(&outside).expect("create outside");
    std::fs::create_dir_all(&out_dir).expect("create out");
    std::os::unix::fs::symlink(&outside, out_dir.join("link")).expect("create symlink");

    let err = extract_tar_gz_to_dir(&tar_path, &out_dir)
        .expect_err("symlink ancestor escape should fail");
    assert!(
        err.to_string().contains("escaped extraction root"),
        "unexpected error: {err:#}"
    );
    assert!(!outside.join("file").exists());
}
