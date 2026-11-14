//! ExifTool as a sidecar (Phase 8 spec §5): the one program that reads and writes every
//! metadata family in every format Lumen exports, and translates between them.
//!
//! Like FFmpeg, a missing ExifTool is a *state*: `ExifTool::locate()` returns `None` and an
//! export says metadata could not be carried rather than dropping it silently.
//!
//! A [`Session`] keeps one ExifTool running in `-stay_open` mode, so a batch pays Perl's
//! start-up once. Arguments go through its `-@ -` argument file, one per line; each command
//! ends with `-echo4 {readyN}` (after processing, on stderr) and `-executeN` (on stdout),
//! so output and errors are both attributed to the command that produced them.

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use crate::ffmpeg::{cmd, TARGET_TRIPLE};
use crate::MediaError;

/// Environment variable naming a directory that holds ExifTool.
pub const ENV_DIR: &str = "LUMEN_EXIFTOOL_DIR";

const EXE: &str = if cfg!(windows) { ".exe" } else { "" };

/// One command may take this long before the session is considered hung.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExifTool {
    pub program: PathBuf,
    /// `exiftool -ver`, e.g. `13.59`.
    pub version: String,
}

impl ExifTool {
    /// Resolution order: `LUMEN_EXIFTOOL_DIR` → next to the executable → `src-tauri/binaries/`
    /// in debug builds → `PATH`. In each directory the sidecar name (`exiftool-<triple>`),
    /// the Perl distribution (`exiftool/exiftool`) and the plain name are tried.
    pub fn locate() -> Option<ExifTool> {
        let mut dirs: Vec<PathBuf> = Vec::new();
        if let Some(d) = std::env::var_os(ENV_DIR) {
            dirs.push(PathBuf::from(d));
        }
        if let Ok(exe) = std::env::current_exe() {
            if let Some(d) = exe.parent() {
                dirs.push(d.to_path_buf());
                // Where the bundles put resources: macOS `Contents/Resources`, Linux
                // `/usr/lib/<product>` beside `/usr/bin` (Windows: next to the exe).
                dirs.push(d.join("../Resources"));
                dirs.push(d.join("../lib/Lumen"));
            }
        }
        if cfg!(debug_assertions) {
            dirs.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../binaries"));
        }
        for dir in dirs {
            for name in [format!("exiftool-{TARGET_TRIPLE}{EXE}"), format!("exiftool/exiftool{EXE}"), format!("exiftool{EXE}")] {
                let p = dir.join(name);
                if p.is_file() {
                    if let Some(e) = Self::at(p) {
                        return Some(e);
                    }
                }
            }
        }
        Self::at(PathBuf::from(format!("exiftool{EXE}")))
    }

    /// The ExifTool at `program`, if it runs and says its version.
    pub fn at(program: PathBuf) -> Option<ExifTool> {
        let out = cmd(&program).arg("-ver").output().ok()?;
        if !out.status.success() {
            return None;
        }
        let version = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let looks_right = version.split('.').next().is_some_and(|major| major.parse::<u32>().is_ok());
        looks_right.then_some(ExifTool { program, version })
    }

    /// Start a long-running session.
    pub fn session(&self) -> Result<Session, MediaError> {
        Session::start(&self.program)
    }
}

/// What one command printed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Output {
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    /// ExifTool reports a failed write as "0 image files updated" plus an error line; a
    /// warning alone ("Warning: …") does not fail the command.
    pub fn errors(&self) -> Vec<&str> {
        self.stderr.lines().map(str::trim).filter(|l| l.starts_with("Error")).collect()
    }
}

/// One ExifTool process in `-stay_open` mode.
pub struct Session {
    child: Child,
    stdin: ChildStdin,
    stdout: Receiver<String>,
    stderr: Receiver<String>,
    next: u32,
}

fn pump(reader: impl std::io::Read + Send + 'static) -> Receiver<String> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(reader).lines() {
            let Ok(line) = line else { break };
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    rx
}

/// Lines from `rx` up to (not including) the line that is exactly `marker`.
pub(crate) fn collect_until(rx: &Receiver<String>, marker: &str, timeout: Duration) -> Result<String, MediaError> {
    let mut out = String::new();
    loop {
        match rx.recv_timeout(timeout) {
            Ok(line) if line.trim_end() == marker => return Ok(out),
            Ok(line) => {
                out.push_str(&line);
                out.push('\n');
            }
            Err(mpsc::RecvTimeoutError::Timeout) => return Err(MediaError::Decode("exiftool: no answer".into())),
            Err(mpsc::RecvTimeoutError::Disconnected) => return Err(MediaError::Decode("exiftool: the process ended".into())),
        }
    }
}

/// One argument as a line of an ExifTool argument file. A value with a line break (a
/// caption) would otherwise split into two arguments, so it is written with ExifTool's
/// `#[CSTR]` prefix, which turns on C escapes for that line.
pub(crate) fn arg_line(arg: &str) -> String {
    if arg.contains(['\n', '\r']) || arg.starts_with('#') {
        let escaped = arg.replace('\\', "\\\\").replace('\n', "\\n").replace('\r', "\\r");
        format!("#[CSTR]{escaped}")
    } else {
        arg.to_string()
    }
}

impl Session {
    fn start(program: &Path) -> Result<Session, MediaError> {
        let mut c = cmd(program);
        c.args(["-stay_open", "True", "-@", "-"]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = c.spawn()?;
        photos_core::child_jobs::adopt(&child);
        let stdin = child.stdin.take().ok_or_else(|| MediaError::Decode("exiftool: no stdin".into()))?;
        let stdout = pump(child.stdout.take().ok_or_else(|| MediaError::Decode("exiftool: no stdout".into()))?);
        let stderr = pump(child.stderr.take().ok_or_else(|| MediaError::Decode("exiftool: no stderr".into()))?);
        Ok(Session { child, stdin, stdout, stderr, next: 1 })
    }

    /// Run one command. Paths are passed as UTF-8 with `-charset filename=utf8`, which is
    /// what makes non-ASCII file names work on Windows.
    pub fn run<I, S>(&mut self, args: I) -> Result<Output, MediaError>
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        let n = self.next;
        self.next += 1;
        let mut script = String::from("-charset\nfilename=utf8\n");
        for a in args {
            let a: OsString = a.into();
            let a = a.into_string().map_err(|a| MediaError::Unsupported(format!("argument is not UTF-8: {a:?}")))?;
            script.push_str(&arg_line(&a));
            script.push('\n');
        }
        let marker = format!("{{ready{n}}}");
        script.push_str(&format!("-echo4\n{marker}\n-execute{n}\n"));
        self.stdin.write_all(script.as_bytes())?;
        self.stdin.flush()?;
        let stdout = collect_until(&self.stdout, &marker, COMMAND_TIMEOUT)?;
        let stderr = collect_until(&self.stderr, &marker, COMMAND_TIMEOUT)?;
        Ok(Output { stdout, stderr })
    }

    /// Tags of `path` as ExifTool's JSON (`-j -n -G1`: numbers unformatted, groups named).
    pub fn read_json(&mut self, path: &Path, tags: &[&str]) -> Result<serde_json::Value, MediaError> {
        let mut args: Vec<OsString> = vec!["-j".into(), "-n".into(), "-G1".into(), "-a".into()];
        args.extend(tags.iter().map(|t| OsString::from(format!("-{t}"))));
        args.push(path.as_os_str().to_owned());
        let out = self.run(args)?;
        let v: serde_json::Value = serde_json::from_str(&out.stdout).map_err(|e| MediaError::Decode(format!("exiftool json: {e}: {}", out.stderr.trim())))?;
        Ok(v.get(0).cloned().unwrap_or(serde_json::Value::Null))
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.stdin.write_all(b"-stay_open\nFalse\n");
        let _ = self.stdin.flush();
        // Give it a moment to exit on its own, then make sure.
        for _ in 0..20 {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_is_read_up_to_its_own_marker() {
        let (tx, rx) = mpsc::channel();
        for l in ["[{", "  \"Make\": \"Nikon\"", "}]", "{ready1}", "next command"] {
            tx.send(l.to_string()).unwrap();
        }
        assert_eq!(collect_until(&rx, "{ready1}", Duration::from_secs(1)).unwrap(), "[{\n  \"Make\": \"Nikon\"\n}]\n");
        assert_eq!(rx.recv().unwrap(), "next command", "the next command's output is left alone");
    }

    #[test]
    fn a_missing_marker_times_out_and_a_dead_process_says_so() {
        let (tx, rx) = mpsc::channel::<String>();
        assert!(collect_until(&rx, "{ready1}", Duration::from_millis(20)).is_err());
        drop(tx);
        let e = collect_until(&rx, "{ready1}", Duration::from_secs(1)).unwrap_err();
        assert!(e.to_string().contains("ended"), "{e}");
    }

    #[test]
    fn a_value_with_line_breaks_stays_one_argument() {
        assert_eq!(arg_line("-Make=Nikon"), "-Make=Nikon");
        assert_eq!(arg_line("-XMP-dc:Description=one\ntwo \\ three"), "#[CSTR]-XMP-dc:Description=one\\ntwo \\\\ three");
        // A line starting with # is a comment in an argument file.
        assert_eq!(arg_line("#hashtag"), "#[CSTR]#hashtag");
    }

    #[test]
    fn errors_are_the_error_lines_only() {
        let o = Output { stdout: String::new(), stderr: "Warning: [minor] odd tag\nError: File not found - x.jpg\n".into() };
        assert_eq!(o.errors(), vec!["Error: File not found - x.jpg"]);
    }

    /// `LUMEN_EXIFTOOL_DIR` or the debug fallback finds it.
    fn exiftool() -> ExifTool {
        ExifTool::locate().expect("ExifTool not found: run scripts/fetch-exiftool or set LUMEN_EXIFTOOL_DIR")
    }

    #[test]
    #[ignore = "needs ExifTool"]
    fn a_session_writes_and_reads_back_through_one_process() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pic ö.jpg");
        image::RgbImage::from_pixel(8, 8, image::Rgb([200, 100, 50])).save(&path).unwrap();
        let mut s = exiftool().session().unwrap();
        let out = s.run(["-overwrite_original", "-XMP-dc:Description=first\nsecond", "-IFD0:Make=Nikon"].into_iter().map(OsString::from).chain([path.clone().into_os_string()])).unwrap();
        assert!(out.errors().is_empty(), "{out:?}");
        let v = s.read_json(&path, &["Make", "Description"]).unwrap();
        assert_eq!(v["IFD0:Make"], "Nikon");
        assert_eq!(v["XMP-dc:Description"], "first\nsecond");
        // A second command on the same process.
        let v = s.read_json(&path, &["Make"]).unwrap();
        assert_eq!(v["IFD0:Make"], "Nikon");
    }
}
