//! Pronunciation: play a Wiktionary recording, or fall back to the
//! operating system's text-to-speech. Uses only tools that ship with the
//! OS (PowerShell on Windows, `afplay`/`say` on macOS) or common Linux
//! players, so the app needs no native audio libraries.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::library::USER_AGENT;

/// Speak `word` (language `lang`), preferring the recordings in `audio`.
/// Runs in a background thread; errors are reported through `on_error`.
pub fn pronounce(word: String, lang: String, audio: Vec<String>, cache: PathBuf, on_error: impl FnOnce(String) + Send + 'static) {
    std::thread::spawn(move || {
        let mut err = String::new();
        for url in &audio {
            match fetch_audio(url, &cache).and_then(|p| play_file(&p)) {
                Ok(()) => return,
                Err(e) => err = e,
            }
        }
        if let Err(e) = speak(&word, &lang) {
            on_error(if err.is_empty() { e } else { format!("{err}; {e}") });
        }
    });
}

/// On Linux prefer the original Ogg file (playable by `paplay`).
fn preferred_url(url: &str) -> String {
    if cfg!(target_os = "linux")
        && let Some(rest) = url.strip_suffix(".mp3")
        && let Some(i) = rest.find("/transcoded/")
    {
        // …/transcoded/3/30/File.ogg/File.ogg.mp3 → …/3/30/File.ogg
        let (base, tail) = rest.split_at(i);
        let tail = &tail["/transcoded".len()..];
        if let Some(j) = tail.rfind('/') {
            return format!("{base}{}", &tail[..j]);
        }
    }
    url.to_owned()
}

fn fetch_audio(url: &str, cache: &Path) -> Result<PathBuf, String> {
    let url = preferred_url(url);
    let name: String = url
        .rsplit('/')
        .next()
        .unwrap_or("audio")
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '.' { c } else { '_' })
        .collect();
    let path = cache.join(format!("{:016x}-{name}", hash(&url)));
    if path.exists() {
        return Ok(path);
    }
    let mut resp = ureq::get(&url)
        .header("User-Agent", USER_AGENT)
        .call()
        .map_err(|e| format!("download failed: {e}"))?;
    let bytes = resp
        .body_mut()
        .with_config()
        .limit(8 << 20)
        .read_to_vec()
        .map_err(|e| e.to_string())?;
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(path)
}

fn hash(s: &str) -> u64 {
    // FNV-1a; only used for cache file names.
    s.bytes().fold(0xcbf29ce484222325, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100000001b3))
}

fn run(cmd: &mut Command) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let status = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() { Ok(()) } else { Err(format!("{status}")) }
}

fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

fn play_file(path: &Path) -> Result<(), String> {
    let p = path.to_string_lossy().into_owned();
    if cfg!(target_os = "macos") {
        return run(Command::new("afplay").arg(&p));
    }
    if cfg!(windows) {
        let script = format!(
            "Add-Type -AssemblyName PresentationCore; $p = New-Object System.Windows.Media.MediaPlayer; \
             $p.Open([uri]{}); $i = 0; while (-not $p.NaturalDuration.HasTimeSpan -and $i -lt 50) {{ Start-Sleep -m 100; $i++ }}; \
             $p.Play(); Start-Sleep -m ([int]$p.NaturalDuration.TimeSpan.TotalMilliseconds + 300); $p.Close()",
            ps_quote(&p)
        );
        return run(Command::new("powershell").args(["-NoProfile", "-Command", &script]));
    }
    // Linux / BSD: try common players.
    let players: &[(&str, &[&str])] = &[
        ("paplay", &[]),
        ("pw-play", &[]),
        ("ffplay", &["-nodisp", "-autoexit", "-loglevel", "quiet"]),
        ("mpv", &["--no-video", "--really-quiet"]),
        ("gst-play-1.0", &["-q"]),
    ];
    let mut last = "no audio player found (install pulseaudio-utils, ffmpeg or mpv)".to_owned();
    for (bin, args) in players {
        match run(Command::new(bin).args(*args).arg(&p)) {
            Ok(()) => return Ok(()),
            Err(e) => last = format!("{bin}: {e}"),
        }
    }
    Err(last)
}

fn speak(word: &str, lang: &str) -> Result<(), String> {
    if cfg!(target_os = "macos") {
        // Pick an installed voice for the language (e.g. Zuzana for cs_CZ).
        let voice = Command::new("say").args(["-v", "?"]).output().ok().and_then(|o| {
            String::from_utf8_lossy(&o.stdout).lines().find_map(|l| {
                let (name, rest) = l.split_once("  ")?;
                let locale = rest.split_whitespace().next()?;
                locale.starts_with(&format!("{lang}_")).then(|| name.trim().to_owned())
            })
        });
        let mut cmd = Command::new("say");
        if let Some(v) = voice {
            cmd.args(["-v", &v]);
        }
        return run(cmd.arg(word));
    }
    if cfg!(windows) {
        let script = format!(
            "Add-Type -AssemblyName System.Speech; $s = New-Object System.Speech.Synthesis.SpeechSynthesizer; \
             $v = $s.GetInstalledVoices() | Where-Object {{ $_.VoiceInfo.Culture.TwoLetterISOLanguageName -eq {} }} | Select-Object -First 1; \
             if ($v) {{ $s.SelectVoice($v.VoiceInfo.Name) }}; $s.Speak({})",
            ps_quote(lang),
            ps_quote(word)
        );
        return run(Command::new("powershell").args(["-NoProfile", "-Command", &script]));
    }
    let espeak_lang = if lang == "grc" { "grc" } else { lang };
    run(Command::new("spd-say").args(["-w", "-l", lang, word]))
        .or_else(|_| run(Command::new("espeak-ng").args(["-v", espeak_lang, word])))
        .or_else(|_| run(Command::new("espeak").args(["-v", espeak_lang, word])))
        .map_err(|_| "no speech synthesizer found (install espeak-ng or speech-dispatcher)".to_owned())
}

#[cfg(test)]
mod tests {
    #[test]
    #[cfg(target_os = "linux")]
    fn ogg_from_transcoded_mp3() {
        assert_eq!(
            super::preferred_url("https://upload.wikimedia.org/wikipedia/commons/transcoded/3/30/Cs-a.ogg/Cs-a.ogg.mp3"),
            "https://upload.wikimedia.org/wikipedia/commons/3/30/Cs-a.ogg"
        );
    }
}
