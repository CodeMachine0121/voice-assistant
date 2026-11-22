//! Where the speech model lives and how the default one is fetched.
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const MODEL_FILE: &str = "ggml-small.bin";
const MODEL_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin";
const CHUNK: usize = 64 * 1024;

/// `<data dir>/VoiceAssistant/models/ggml-small.bin`; on macOS the data dir is
/// `~/Library/Application Support`. Falls back to the working directory if no home is known.
pub fn default_model_path() -> PathBuf {
    model_path_in(dirs::data_dir().unwrap_or_default())
}

pub(crate) fn model_path_in(data_dir: PathBuf) -> PathBuf {
    data_dir
        .join("VoiceAssistant")
        .join("models")
        .join(MODEL_FILE)
}

/// Downloads the default model to `dest`. Bytes go to `dest` with extension `part` first and
/// the file is renamed only when complete, so `dest` never holds a partial model.
/// `progress(done, total)` is called as bytes arrive; `total` is unknown if the server omits it.
pub fn download_default_model(
    dest: &Path,
    progress: impl FnMut(u64, Option<u64>),
) -> Result<(), String> {
    let response = ureq::get(MODEL_URL)
        .call()
        .map_err(|e| format!("model download failed: {e}"))?;
    let total = response.body().content_length();
    let reader = response.into_body().into_reader();
    save_atomically(reader, total, dest, progress)
}

/// Streams `reader` into `dest` via a `.part` file, reporting progress after every chunk.
pub(crate) fn save_atomically(
    mut reader: impl Read,
    total: Option<u64>,
    dest: &Path,
    mut progress: impl FnMut(u64, Option<u64>),
) -> Result<(), String> {
    let part = dest.with_extension("part");
    let result = (|| {
        if let Some(dir) = dest.parent() {
            fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        }
        let mut file =
            File::create(&part).map_err(|e| format!("cannot write {}: {e}", part.display()))?;
        let mut buf = vec![0u8; CHUNK];
        let mut done = 0u64;
        progress(0, total);
        loop {
            let n = reader
                .read(&mut buf)
                .map_err(|e| format!("download interrupted: {e}"))?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n])
                .map_err(|e| format!("cannot write model: {e}"))?;
            done += n as u64;
            progress(done, total);
        }
        file.flush()
            .map_err(|e| format!("cannot write model: {e}"))?;
        if total.is_some_and(|t| t != done) {
            return Err(format!(
                "download incomplete: got {done} of {} bytes",
                total.unwrap_or(0)
            ));
        }
        fs::rename(&part, dest).map_err(|e| format!("cannot move model into place: {e}"))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&part);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn model_lives_in_the_voice_assistant_models_folder() {
        let path = model_path_in(PathBuf::from("/Users/me/Library/Application Support"));
        assert_eq!(
            path,
            PathBuf::from(
                "/Users/me/Library/Application Support/VoiceAssistant/models/ggml-small.bin"
            )
        );
    }

    #[test]
    fn default_path_ends_with_the_small_model() {
        assert!(default_model_path().ends_with("VoiceAssistant/models/ggml-small.bin"));
    }

    #[test]
    fn download_writes_the_file_and_leaves_no_part_file() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("models").join("ggml-small.bin");
        save_atomically(
            Cursor::new(vec![7u8; 200_000]),
            Some(200_000),
            &dest,
            |_, _| {},
        )
        .unwrap();
        assert_eq!(fs::read(&dest).unwrap(), vec![7u8; 200_000]);
        assert!(!dest.with_extension("part").exists());
    }

    #[test]
    fn download_reports_progress_up_to_the_total() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("m.bin");
        let mut seen = Vec::new();
        save_atomically(
            Cursor::new(vec![1u8; 100_000]),
            Some(100_000),
            &dest,
            |d, t| seen.push((d, t)),
        )
        .unwrap();
        assert_eq!(seen.first(), Some(&(0, Some(100_000))));
        assert_eq!(seen.last(), Some(&(100_000, Some(100_000))));
        assert!(seen.windows(2).all(|w| w[0].0 <= w[1].0));
    }

    #[test]
    fn download_without_known_size_still_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("m.bin");
        let mut last = None;
        save_atomically(Cursor::new(vec![1u8; 10]), None, &dest, |d, t| {
            last = Some((d, t))
        })
        .unwrap();
        assert_eq!(last, Some((10, None)));
        assert_eq!(fs::read(&dest).unwrap().len(), 10);
    }

    #[test]
    fn short_download_is_rejected_and_nothing_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("m.bin");
        let err =
            save_atomically(Cursor::new(vec![1u8; 10]), Some(20), &dest, |_, _| {}).unwrap_err();
        assert!(err.contains("incomplete"), "{err}");
        assert!(!dest.exists() && !dest.with_extension("part").exists());
    }

    struct Broken;
    impl Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("connection reset"))
        }
    }

    #[test]
    fn interrupted_download_keeps_an_existing_model_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("m.bin");
        fs::write(&dest, b"old").unwrap();
        let err = save_atomically(Broken, None, &dest, |_, _| {}).unwrap_err();
        assert!(err.contains("connection reset"), "{err}");
        assert_eq!(fs::read(&dest).unwrap(), b"old");
        assert!(!dest.with_extension("part").exists());
    }

    #[test]
    fn unwritable_destination_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("file");
        fs::write(&blocker, b"x").unwrap();
        let dest = blocker.join("sub").join("m.bin"); // parent would have to be inside a file
        assert!(save_atomically(Cursor::new(vec![1u8]), None, &dest, |_, _| {}).is_err());
    }

    #[test]
    fn part_file_that_cannot_be_created_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("m.bin");
        fs::create_dir(dest.with_extension("part")).unwrap(); // a directory is in the way
        let err = save_atomically(Cursor::new(vec![1u8]), None, &dest, |_, _| {}).unwrap_err();
        assert!(err.contains("cannot write"), "{err}");
    }

    #[test]
    fn destination_that_cannot_be_replaced_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("m.bin");
        fs::create_dir(&dest).unwrap();
        fs::write(dest.join("keep"), b"x").unwrap(); // non-empty directory blocks the rename
        let err = save_atomically(Cursor::new(vec![1u8]), None, &dest, |_, _| {}).unwrap_err();
        assert!(err.contains("cannot move"), "{err}");
    }
}
