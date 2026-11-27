//! The installed fonts as on-demand candidates: the font folders' files by name, the CJK lists per platform, and
//! Linux's fontconfig lookup for the CJK scripts.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use craft_font_catalog::{FontEnv, font_dirs, is_font_file};
use craft_host::Wake;
use craft_host::target::Os;
use craft_i18n::Han;

use super::{MAX_FONT_BYTES, covering_face, is_cjk};

/// At most this many name matches per script.
const MAX_CANDIDATES: usize = 16;
/// Folders nested deeper than this are not listed.
const MAX_DEPTH: usize = 8;

/// A CJK writing system with its own installed fonts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cjk {
    Japanese,
    Simplified,
    Traditional,
    Korean,
}

/// The order in which the regional faces are tried: the preferred region first.
fn order(han: Option<Han>) -> [Cjk; 4] {
    use Cjk::*;
    match han {
        Some(Han::Japanese) => [Japanese, Simplified, Traditional, Korean],
        Some(Han::Korean) => [Korean, Traditional, Simplified, Japanese],
        Some(Han::Traditional | Han::HongKong) => [Traditional, Simplified, Japanese, Korean],
        _ => [Simplified, Traditional, Japanese, Korean],
    }
}

pub(super) fn files(script: &str, han: Option<Han>) -> Vec<PathBuf> {
    if !is_cjk(script) {
        return candidates(script, listing());
    }
    let order = order(han);
    let first = match script {
        "Kana" => Cjk::Japanese,
        "Hang" => Cjk::Korean,
        "Bopo" => Cjk::Traditional,
        _ => order[0],
    };
    let mut files: Vec<PathBuf> = Vec::new();
    for region in std::iter::once(first).chain(order) {
        for path in cjk_files(region) {
            if path.is_file() && !files.contains(&path) {
                files.push(path);
            }
        }
    }
    for path in last_resort_files() {
        if path.is_file() && !files.contains(&path) {
            files.push(path);
        }
    }
    files
}

pub(super) fn wake_when_ready(wake: Wake) {
    if Os::current() == Os::Linux {
        fontconfig::wake_when_ready(wake);
    } else {
        wake.wake();
    }
}

pub(super) fn pending() -> bool {
    if Os::current() == Os::Linux {
        fontconfig::start();
        fontconfig::pending()
    } else {
        false
    }
}

/// System CJK font files for `region`, most preferred first. Paths that don't exist are
/// included; callers skip them.
fn cjk_files(region: Cjk) -> Vec<PathBuf> {
    match Os::current() {
        Os::Mac => mac_cjk_files(region),
        Os::Windows => windows_cjk_files(region),
        _ => linux_cjk_files(region),
    }
}

fn mac_cjk_files(region: Cjk) -> Vec<PathBuf> {
    use Cjk::*;
    let p = |path: &str| PathBuf::from(path);
    let pingfang = mac_pingfang();
    match region {
        Japanese => vec![p("/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc"), p("/System/Library/Fonts/ヒラギノ角ゴシック W4.ttc")],
        Simplified => {
            pingfang.into_iter().chain([p("/System/Library/Fonts/Hiragino Sans GB.ttc"), p("/System/Library/Fonts/STHeiti Light.ttc")]).collect()
        }
        Traditional => pingfang.into_iter().chain([p("/System/Library/Fonts/STHeiti Light.ttc")]).collect(),
        Korean => vec![p("/System/Library/Fonts/AppleSDGothicNeo.ttc")],
    }
}

fn windows_cjk_files(region: Cjk) -> Vec<PathBuf> {
    use Cjk::*;
    let dir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
    let w = |file: &str| Path::new(&dir).join("Fonts").join(file);
    match region {
        Japanese => vec![w("YuGothR.ttc"), w("YuGothM.ttc"), w("meiryo.ttc"), w("msgothic.ttc")],
        Simplified => vec![w("msyh.ttc"), w("msyh.ttf"), w("simsun.ttc")],
        Traditional => vec![w("msjh.ttc"), w("msjh.ttf"), w("mingliu.ttc")],
        Korean => vec![w("malgun.ttf"), w("gulim.ttc")],
    }
}

fn linux_cjk_files(region: Cjk) -> Vec<PathBuf> {
    use Cjk::*;
    let p = |path: &str| PathBuf::from(path);
    // One Noto Sans CJK collection covers all four regions; the loader picks the face.
    let mut files: Vec<PathBuf> = [
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/local/share/fonts/noto/NotoSansCJK-Regular.ttc",
        "/usr/local/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
        "/usr/local/share/fonts/noto/NotoSansCJK-VF.ttc",
        "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/google-noto-sans-cjk-fonts/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/google-noto-sans-cjk-vf-fonts/NotoSansCJK-VF.ttc",
        "/usr/share/fonts/noto-cjk/NotoSansCJK-VF.ttc",
    ]
    .map(p)
    .to_vec();
    files.extend(
        match region {
            Japanese => &[
                "/usr/share/fonts/opentype/ipafont-gothic/ipag.ttf",
                "/usr/share/fonts/truetype/fonts-japanese-gothic.ttf",
                "/usr/share/fonts/truetype/takao-gothic/TakaoPGothic.ttf",
            ][..],
            Simplified | Traditional => &[
                "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
                "/usr/share/fonts/wenquanyi/wqy-microhei/wqy-microhei.ttc",
                "/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc",
                "/usr/share/fonts/wenquanyi/wqy-zenhei/wqy-zenhei.ttc",
            ][..],
            Korean => &[
                "/usr/share/fonts/truetype/nanum/NanumGothic.ttf",
                "/usr/share/fonts/nanum/NanumGothic.ttf",
                "/usr/share/fonts/truetype/unfonts-core/UnDotum.ttf",
                "/usr/share/fonts/un-core/UnDotum.ttf",
            ][..],
        }
        .iter()
        .map(|f| p(f)),
    );
    // Flatpak mounts the host's font folder outside the runtime's.
    let mut with_host = Vec::new();
    for file in files {
        let host = file.strip_prefix("/usr/share/fonts").ok().map(|relative| Path::new("/run/host/fonts").join(relative));
        with_host.push(file);
        with_host.extend(host);
    }
    if let Some(font) = fontconfig::result(region)
        && !with_host.contains(&font)
    {
        with_host.push(font);
    }
    with_host
}

/// Broad-coverage fonts tried after every region's own: they cover Han, kana and Hangul but
/// look worse than the regional fonts.
fn last_resort_files() -> Vec<PathBuf> {
    let p = |path: &str| PathBuf::from(path);
    match Os::current() {
        Os::Mac => vec![p("/System/Library/Fonts/Supplemental/Arial Unicode.ttf"), p("/Library/Fonts/Arial Unicode.ttf")],
        Os::Windows => {
            let dir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
            vec![Path::new(&dir).join("Fonts").join("ARIALUNI.TTF")]
        }
        _ => vec![
            p("/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf"),
            p("/usr/share/fonts/google-droid-sans-fonts/DroidSansFallbackFull.ttf"),
            p("/run/host/fonts/truetype/droid/DroidSansFallbackFull.ttf"),
            p("/run/host/fonts/google-droid-sans-fonts/DroidSansFallbackFull.ttf"),
        ],
    }
}

/// macOS ships PingFang as a pre-installed asset outside `/System/Library/Fonts`
/// (`/System/Library/AssetsV2/com_apple_MobileAsset_Font*/<hash>.asset/AssetData/PingFang.ttc`).
fn mac_pingfang() -> Option<PathBuf> {
    let old = PathBuf::from("/System/Library/Fonts/PingFang.ttc");
    if old.is_file() {
        return Some(old);
    }
    let roots = [Path::new("/System/Library/AssetsV2"), Path::new("/System/Library/AssetsV2/PreinstalledAssetsV2/InstallWithOs")];
    for root in roots {
        let Ok(entries) = std::fs::read_dir(root) else { continue };
        for entry in entries.flatten().take(512) {
            if !entry.file_name().to_string_lossy().starts_with("com_apple_MobileAsset_Font") {
                continue;
            }
            let Ok(assets) = std::fs::read_dir(entry.path()) else { continue };
            for asset in assets.flatten().take(64) {
                let path = asset.path().join("AssetData/PingFang.ttc");
                if path.is_file() {
                    return Some(path);
                }
            }
        }
    }
    None
}

/// Every font file in the platform's font folders, listed on first use and kept for the
/// process (names only; nothing is opened).
fn listing() -> &'static [PathBuf] {
    static FILES: OnceLock<Vec<PathBuf>> = OnceLock::new();
    FILES.get_or_init(|| {
        let mut files = Vec::new();
        for dir in font_dirs(&FontEnv::from_process()) {
            collect(&dir, 0, &mut files);
        }
        files.sort();
        files.dedup();
        files
    })
}

fn collect(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > MAX_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, depth + 1, out);
        } else if is_font_file(&path) {
            out.push(path);
        }
    }
}

/// Font file names for a non-CJK script, most preferred first: lowercase, without extension,
/// spaces, underscores or style suffix (`NotoSansThai-Bold.ttf` and `NotoSansThai[wght].ttf`
/// are both `notosansthai`). Noto first (the usual Linux font), then Windows, macOS and other
/// Linux families.
fn file_names(script: &str) -> &'static [&'static str] {
    match script {
        "Thai" => &[
            "notosansthai",
            "notosansthailooped",
            "notosansthaiui",
            "notoserifthai",
            "leelawui",
            "leelawad",
            "tahoma",
            "thonburi",
            "sukhumvitset",
            "ayuthaya",
            "loma",
            "garuda",
            "waree",
            "kinnari",
            "norasi",
        ],
        "Laoo" => &["notosanslao", "notosanslaoui", "notoseriflao", "leelawui", "laosangammn", "laomn", "phetsarathot", "saysetthaot"],
        "Khmr" => {
            &["notosanskhmer", "notosanskhmerui", "notoserifkhmer", "leelawui", "daunpenh", "khmersangammn", "khmermn", "khmeros", "khmerossys"]
        }
        "Mymr" => &["notosansmyanmar", "notosansmyanmarui", "notoserifmyanmar", "mmrtext", "myanmarsangammn", "myanmarmn", "padauk"],
        "Tibt" => &["notoseriftibetan", "himalaya", "kailasa", "jomolhari", "tibetanmachineuni"],
        "Arab" => &[
            "notosansarabic",
            "notosansarabicui",
            "notonaskharabic",
            "notonaskharabicui",
            "notokufiarabic",
            "segoeui",
            "tahoma",
            "geezapro",
            "sfarabic",
            "dejavusans",
            "kacstone",
            "kacstbook",
            "amiri",
            "arial",
        ],
        "Hebr" => &["notosanshebrew", "notoserifhebrew", "segoeui", "arialhb", "sfhebrew", "dejavusans", "arial", "freesans", "culmus"],
        "Armn" => &["notosansarmenian", "notoserifarmenian", "segoeui", "sylfaen", "mshtakan", "dejavusans", "freesans"],
        "Geor" => &["notosansgeorgian", "notoserifgeorgian", "segoeui", "sylfaen", "dejavusans", "freesans"],
        "Ethi" => &["notosansethiopic", "notoserifethiopic", "ebrima", "kefa", "abyssinicasil"],
        "Deva" => &[
            "notosansdevanagari",
            "notosansdevanagariui",
            "notoserifdevanagari",
            "nirmala",
            "mangal",
            "kohinoor",
            "kohinoordevanagari",
            "itfdevanagari",
            "devanagarimt",
            "devanagarisangammn",
            "lohitdevanagari",
            "lohit-devanagari",
            "gargi",
            "freesans",
        ],
        "Beng" => &[
            "notosansbengali",
            "notosansbengaliui",
            "notoserifbengali",
            "nirmala",
            "vrinda",
            "kohinoorbangla",
            "banglasangammn",
            "banglamn",
            "lohitbengali",
            "lohit-bengali",
            "lohit-assamese",
        ],
        "Guru" => &[
            "notosansgurmukhi",
            "notosansgurmukhiui",
            "notoserifgurmukhi",
            "nirmala",
            "raavi",
            "gurmukhisangammn",
            "gurmukhimn",
            "lohit-gurmukhi",
            "lohit-punjabi",
        ],
        "Gujr" => &[
            "notosansgujarati",
            "notosansgujaratiui",
            "notoserifgujarati",
            "nirmala",
            "shruti",
            "kohinoorgujarati",
            "gujaratisangammn",
            "gujaratimt",
            "lohit-gujarati",
        ],
        "Orya" => {
            &["notosansoriya", "notosansoriyaui", "notoseriforiya", "nirmala", "kalinga", "oriyasangammn", "oriyamn", "lohit-odia", "lohit-oriya"]
        }
        "Taml" => &["notosanstamil", "notosanstamilui", "notoseriftamil", "nirmala", "latha", "tamilsangammn", "tamilmn", "lohit-tamil"],
        "Telu" => &[
            "notosanstelugu",
            "notosansteluguui",
            "notoseriftelugu",
            "nirmala",
            "gautami",
            "kohinoortelugu",
            "telugusangammn",
            "telugumn",
            "lohit-telugu",
        ],
        "Knda" => &["notosanskannada", "notosanskannadaui", "notoserifkannada", "nirmala", "tunga", "kannadasangammn", "kannadamn", "lohit-kannada"],
        "Mlym" => &[
            "notosansmalayalam",
            "notosansmalayalamui",
            "notoserifmalayalam",
            "nirmala",
            "kartika",
            "malayalamsangammn",
            "malayalammn",
            "lohit-malayalam",
        ],
        "Sinh" => &["notosanssinhala", "notosanssinhalaui", "notoserifsinhala", "nirmala", "iskpota", "sinhalasangammn", "sinhalamn", "lklug"],
        _ => &[],
    }
}

/// `(name, stem, style rank)` of a font file: [`file_names`] form, the file stem, and 0 for the
/// regular face, 1 for a variable font, 2 for other weights, 3 for bold, italic and condensed
/// faces. Non-font files give `None`.
fn file_key(path: &Path) -> Option<(String, &str, u8)> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    if !matches!(ext.as_str(), "ttf" | "otf" | "ttc" | "otc") {
        return None;
    }
    let stem = path.file_stem()?.to_str()?;
    let squash = |s: &str| s.chars().filter(|c| !matches!(c, ' ' | '_')).collect::<String>().to_lowercase();
    let style = |s: &str| {
        let s = s.to_ascii_lowercase();
        if s.is_empty() || s == "-regular" {
            0
        } else if s.starts_with('[') {
            1
        } else if ["bold", "italic", "oblique", "condensed", "narrow", "black", "heavy", "thin", "light"].iter().any(|w| s.contains(w)) {
            3
        } else {
            2
        }
    };
    let cut = stem.find(['-', '[']).unwrap_or(stem.len());
    let (base, rest) = stem.split_at(cut);
    Some((squash(base), stem, style(rest)))
}

/// Font files among `files` that may cover `script`, most preferred first. Pure, so tests
/// pass their own listing.
pub(super) fn candidates(script: &str, files: &[PathBuf]) -> Vec<PathBuf> {
    let names = file_names(script);
    let mut ranked: Vec<(usize, u8, &PathBuf)> = files
        .iter()
        .filter_map(|path| {
            let (base, stem, style) = file_key(path)?;
            // A name with a hyphen ("lohit-tamil") matches the whole stem.
            let full = stem.to_lowercase().replace([' ', '_'], "");
            let rank = names.iter().position(|n| *n == base || *n == full)?;
            Some((rank, if names.get(rank).is_some_and(|n| *n == full) { 0 } else { style }, path))
        })
        .collect();
    ranked.sort();
    ranked.into_iter().take(MAX_CANDIDATES).map(|(.., path)| path.clone()).collect()
}

/// Linux's fontconfig choice per CJK region, found by `fc-match` on a background thread.
mod fontconfig {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Mutex, OnceLock, PoisonError};

    use craft_host::Wake;

    use super::{Cjk, MAX_FONT_BYTES, covering_face};

    struct Scan {
        started: AtomicBool,
        state: Mutex<State>,
    }

    #[derive(Default)]
    struct State {
        result: Option<[Option<PathBuf>; 4]>,
        /// Woken, then dropped, when the result arrives.
        wakes: Vec<Wake>,
    }

    impl Scan {
        fn finish(&self, result: [Option<PathBuf>; 4]) {
            let wakes = {
                let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
                state.result = Some(result);
                std::mem::take(&mut state.wakes)
            };
            wakes.iter().for_each(Wake::wake);
        }
    }

    fn scan() -> &'static Scan {
        static SCAN: OnceLock<Scan> = OnceLock::new();
        SCAN.get_or_init(|| Scan { started: AtomicBool::new(false), state: Mutex::default() })
    }

    const REGIONS: [Cjk; 4] = [Cjk::Japanese, Cjk::Simplified, Cjk::Traditional, Cjk::Korean];

    /// Start the lookup; later calls do nothing. Without `fc-match`, the built-in paths stand.
    pub(super) fn start() {
        let scan = scan();
        if scan.started.swap(true, Ordering::Relaxed) {
            return;
        }
        let spawned = std::thread::Builder::new().name("craft-ui-fontconfig-cjk".into()).spawn(move || {
            scan.finish(REGIONS.map(fc_match));
        });
        if spawned.is_err() {
            scan.finish([None, None, None, None]);
        }
    }

    /// Whether the lookup has started and is still running.
    pub(super) fn pending() -> bool {
        let scan = scan();
        scan.started.load(Ordering::Relaxed) && scan.state.lock().unwrap_or_else(PoisonError::into_inner).result.is_none()
    }

    /// Call `wake` when the lookup ends, at once if it has.
    pub(super) fn wake_when_ready(wake: Wake) {
        let mut state = scan().state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.result.is_some() {
            drop(state);
            wake.wake();
        } else {
            state.wakes.push(wake);
        }
    }

    pub(super) fn result(region: Cjk) -> Option<PathBuf> {
        let scan = scan();
        if !scan.started.load(Ordering::Relaxed) {
            return None;
        }
        let index = REGIONS.iter().position(|r| *r == region)?;
        scan.state.lock().unwrap_or_else(PoisonError::into_inner).result.as_ref()?.get(index)?.clone()
    }

    /// The font `fc-match :lang=<region>` names, if it is a font file with every character of
    /// the region's sample.
    fn fc_match(region: Cjk) -> Option<PathBuf> {
        let (language, sample) = match region {
            Cjk::Japanese => ("ja", "レイヤー図"),
            Cjk::Simplified => ("zh-cn", "图层"),
            Cjk::Traditional => ("zh-tw", "圖層"),
            Cjk::Korean => ("ko", "카드 배경"),
        };
        let output = std::process::Command::new("fc-match").args(["--format=%{file}\n", &format!(":lang={language}")]).output().ok()?;
        if !output.status.success() {
            return None;
        }
        let line = String::from_utf8(output.stdout).ok()?;
        let path = PathBuf::from(line.lines().find(|l| !l.is_empty())?);
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        if !path.is_absolute() || !matches!(ext.as_str(), "ttf" | "otf" | "ttc" | "otc") {
            return None;
        }
        let meta = std::fs::metadata(&path).ok()?;
        if !meta.is_file() || meta.len() == 0 || meta.len() > MAX_FONT_BYTES {
            return None;
        }
        let bytes = std::fs::read(&path).ok()?;
        sample.chars().all(|c| covering_face(&bytes, c).is_some()).then(|| path.canonicalize().unwrap_or(path))
    }
}
