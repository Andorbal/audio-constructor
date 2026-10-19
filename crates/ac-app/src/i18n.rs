//! The editor's own words, translated (docs/TRANSLATING.md): menus, panel
//! and tab titles, the transport bar, dialogs, the status line, and the
//! help and settings chrome. Node names, node docs, and project content
//! aren't translated yet.
//!
//! Text comes from Fluent catalogs (`locales/<language>/*.ftl`), embedded
//! in the binary so the browser build needs no files. [`tr`] and
//! [`tr_args`] look a message up in the language this thread shows, the
//! way the theme module's palette is current: per thread, so tests side
//! by side can't change each other's language. A message a catalog lacks
//! falls back to English, and one no catalog has shows its id; nothing
//! here panics.
//!
//! **Pseudo** is a made-up language generated from English at run time:
//! every letter accented, about a third longer, and wrapped in brackets,
//! so text that isn't translated (no brackets) and text that won't fit
//! (cut off, or overflowing) stand out.

use std::borrow::Cow;
use std::cell::Cell;
use std::sync::OnceLock;

use fluent_bundle::concurrent::FluentBundle;
use fluent_bundle::{FluentArgs, FluentResource, FluentValue};
use unic_langid::LanguageIdentifier;

/// The English catalog, file by file (the source every other language
/// translates, and the fallback for messages they lack).
pub const ENGLISH: &[(&str, &str)] = &[
    ("app.ftl", include_str!("../locales/en/app.ftl")),
    ("canvas.ftl", include_str!("../locales/en/canvas.ftl")),
    ("menus.ftl", include_str!("../locales/en/menus.ftl")),
    ("undo.ftl", include_str!("../locales/en/undo.ftl")),
];

/// A language the editor can show.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Language {
    #[default]
    English,
    /// English made strange on purpose, to find what isn't translated.
    Pseudo,
}

impl Language {
    /// The language a system locale (`de-DE`, `en_GB.UTF-8`) asks for, if
    /// the editor has it. Pseudo is never a system's.
    pub fn for_locale(locale: &str) -> Option<Language> {
        let primary = locale
            .split(['-', '_', '.', '@'])
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        match primary.as_str() {
            "en" => Some(Language::English),
            _ => None,
        }
    }

    fn id(self) -> LanguageIdentifier {
        // Plural rules are looked up by exact tag, and the tables have
        // plain `en`. Pseudo counts like English, the text it's made from.
        let tag = match self {
            Language::English | Language::Pseudo => "en",
        };
        tag.parse().unwrap_or_default()
    }
}

/// What View ▸ Language picks, kept in the settings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LanguageChoice {
    /// The operating system's language, or English if the editor doesn't
    /// have it.
    #[default]
    System,
    English,
    Pseudo,
}

impl LanguageChoice {
    pub const ALL: [LanguageChoice; 3] = [
        LanguageChoice::System,
        LanguageChoice::English,
        LanguageChoice::Pseudo,
    ];

    /// The language this choice shows here.
    pub fn resolve(self) -> Language {
        match self {
            LanguageChoice::System => system_language(),
            LanguageChoice::English => Language::English,
            LanguageChoice::Pseudo => Language::Pseudo,
        }
    }

    /// The name View ▸ Language shows.
    pub fn label(self) -> String {
        match self {
            LanguageChoice::System => tr("language-system"),
            LanguageChoice::English => tr("language-english"),
            LanguageChoice::Pseudo => tr("language-pseudo"),
        }
    }
}

/// The system's language if the editor has it, else English. The browser
/// build reports no system language, so it shows English.
pub fn system_language() -> Language {
    sys_locale::get_locales()
        .find_map(|l| Language::for_locale(&l))
        .unwrap_or_default()
}

thread_local! {
    /// The language this thread's UI shows.
    static CURRENT: Cell<Language> = const { Cell::new(Language::English) };
}

/// The language this thread shows.
pub fn language() -> Language {
    CURRENT.with(Cell::get)
}

/// Show `lang` from now on (on this thread).
pub fn set_language(lang: Language) {
    CURRENT.with(|c| c.set(lang));
}

/// A value for a message's `{ $name }`: text, or a count (which picks a
/// plural form). Other numbers are formatted by the caller and passed as
/// text, so units and precision stay where they're decided.
#[derive(Clone, Debug, PartialEq)]
pub enum Arg<'a> {
    Text(Cow<'a, str>),
    Count(i64),
}

impl<'a> From<&'a str> for Arg<'a> {
    fn from(s: &'a str) -> Self {
        Arg::Text(Cow::Borrowed(s))
    }
}

impl<'a> From<&'a String> for Arg<'a> {
    fn from(s: &'a String) -> Self {
        Arg::Text(Cow::Borrowed(s.as_str()))
    }
}

impl From<String> for Arg<'_> {
    fn from(s: String) -> Self {
        Arg::Text(Cow::Owned(s))
    }
}

impl From<usize> for Arg<'_> {
    fn from(n: usize) -> Self {
        Arg::Count(i64::try_from(n).unwrap_or(i64::MAX))
    }
}

impl From<u32> for Arg<'_> {
    fn from(n: u32) -> Self {
        Arg::Count(i64::from(n))
    }
}

impl From<u64> for Arg<'_> {
    fn from(n: u64) -> Self {
        Arg::Count(i64::try_from(n).unwrap_or(i64::MAX))
    }
}

impl From<i32> for Arg<'_> {
    fn from(n: i32) -> Self {
        Arg::Count(i64::from(n))
    }
}

impl From<i64> for Arg<'_> {
    fn from(n: i64) -> Self {
        Arg::Count(n)
    }
}

/// The message `id` in this thread's language.
pub fn tr(id: &str) -> String {
    format(language(), id, None)
}

/// The message `id` with its `{ $name }`s filled in.
pub fn tr_args(id: &str, args: &[(&str, Arg<'_>)]) -> String {
    let mut fa = FluentArgs::with_capacity(args.len());
    for (name, value) in args {
        let v = match value {
            Arg::Text(s) => FluentValue::from(s.as_ref()),
            Arg::Count(n) => FluentValue::from(*n),
        };
        fa.set(*name, v);
    }
    format(language(), id, Some(&fa))
}

/// Whether the English catalog has `id` (tests and tools).
pub fn has_message(id: &str) -> bool {
    bundles().english.has_message(id)
}

struct Bundles {
    english: FluentBundle<&'static FluentResource>,
    pseudo: FluentBundle<&'static FluentResource>,
}

fn bundles() -> &'static Bundles {
    static BUNDLES: OnceLock<Bundles> = OnceLock::new();
    BUNDLES.get_or_init(|| {
        let english = resources(ENGLISH);
        Bundles {
            english: bundle(Language::English, english, None),
            pseudo: bundle(Language::Pseudo, english, Some(pseudo_text)),
        }
    })
}

/// A catalog's files, parsed once for the whole run. A file with syntax
/// errors keeps the messages that did parse (a test fails on any error).
fn resources(files: &'static [(&'static str, &'static str)]) -> &'static [FluentResource] {
    let parsed: Vec<FluentResource> = files
        .iter()
        .map(
            |(_, text)| match FluentResource::try_new((*text).to_string()) {
                Ok(r) => r,
                Err((r, _)) => r,
            },
        )
        .collect();
    Vec::leak(parsed)
}

fn bundle(
    lang: Language,
    resources: &'static [FluentResource],
    transform: Option<fn(&str) -> Cow<'_, str>>,
) -> FluentBundle<&'static FluentResource> {
    let mut b = FluentBundle::new_concurrent(vec![lang.id()]);
    // Unicode isolation marks around arguments would show as boxes in the
    // editor's fonts; the editor's text runs left to right.
    b.set_use_isolating(false);
    b.set_transform(transform);
    for r in resources {
        // Later files can't redefine a message (a test checks); the first
        // definition wins.
        let _ = b.add_resource(r);
    }
    b
}

fn format(lang: Language, id: &str, args: Option<&FluentArgs>) -> String {
    let all = bundles();
    let (bundle, pseudo) = match lang {
        Language::English => (&all.english, false),
        Language::Pseudo => (&all.pseudo, true),
    };
    let Some(pattern) = bundle
        .get_message(id)
        .or_else(|| all.english.get_message(id))
        .and_then(|m| m.value())
    else {
        // A message no catalog has: its id shows, so it's easy to spot.
        return id.to_string();
    };
    let mut errors = Vec::new();
    let text = bundle
        .format_pattern(pattern, args, &mut errors)
        .into_owned();
    if !pseudo {
        return text;
    }
    // About 35 % longer than the English, brackets included, so labels
    // that only just fit in English show where they'd overflow.
    let english = all
        .english
        .format_pattern(pattern, args, &mut errors)
        .chars()
        .count();
    let pad = (english * 35).div_ceil(100).saturating_sub(2);
    let mut out = String::with_capacity(text.len() + pad + 2);
    out.push('[');
    out.push_str(&text);
    out.extend(std::iter::repeat_n('·', pad));
    out.push(']');
    out
}

/// Pseudo's letters: each ASCII letter swapped for an accented one the
/// editor's fonts have (Latin-1 and Latin Extended-A), so text stays
/// readable while anything not translated stands out.
fn pseudo_text(s: &str) -> Cow<'_, str> {
    Cow::Owned(s.chars().map(pseudo_char).collect())
}

fn pseudo_char(c: char) -> char {
    match c {
        'A' => 'Å',
        'C' => 'Ç',
        'D' => 'Ď',
        'E' => 'É',
        'G' => 'Ĝ',
        'H' => 'Ĥ',
        'I' => 'Î',
        'J' => 'Ĵ',
        'K' => 'Ķ',
        'L' => 'Ľ',
        'N' => 'Ñ',
        'O' => 'Ö',
        'R' => 'Ŕ',
        'S' => 'Š',
        'T' => 'Ť',
        'U' => 'Û',
        'W' => 'Ŵ',
        'Y' => 'Ý',
        'Z' => 'Ž',
        'a' => 'å',
        'c' => 'ç',
        'd' => 'ď',
        'e' => 'é',
        'g' => 'ĝ',
        'h' => 'ĥ',
        'i' => 'î',
        'j' => 'ĵ',
        'k' => 'ķ',
        'l' => 'ľ',
        'n' => 'ñ',
        'o' => 'ö',
        'r' => 'ŕ',
        's' => 'š',
        't' => 'ť',
        'u' => 'û',
        'w' => 'ŵ',
        'y' => 'ý',
        'z' => 'ž',
        other => other,
    }
}

/// Whether `text` came from the Pseudo catalog: it holds a bracketed run
/// (tests use this to find text that isn't translated).
pub fn is_pseudo(text: &str) -> bool {
    match (text.find('['), text.rfind(']')) {
        (Some(a), Some(b)) => a < b,
        _ => false,
    }
}

/// A library category's name, as the editor shows it.
pub fn category(cat: ac_core::Category) -> String {
    use ac_core::Category;
    match cat {
        Category::Io => tr("category-io"),
        Category::Generators => tr("category-generators"),
        Category::Level => tr("category-level"),
        Category::Filters => tr("category-filters"),
        Category::Time => tr("category-time"),
        Category::Spectral => tr("category-spectral"),
        Category::Math => tr("category-math"),
        Category::Modulation => tr("category-modulation"),
        Category::Triggers => tr("category-triggers"),
        Category::Sequencing => tr("category-sequencing"),
        Category::Controls => tr("category-controls"),
        Category::Display => tr("category-display"),
        Category::Utility => tr("category-utility"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shows `lang` on this thread until dropped.
    struct Showing;
    impl Showing {
        fn language(lang: Language) -> Showing {
            set_language(lang);
            Showing
        }
    }
    impl Drop for Showing {
        fn drop(&mut self) {
            set_language(Language::English);
        }
    }

    #[test]
    fn english_is_the_default_and_fills_in_arguments() {
        assert_eq!(language(), Language::English);
        assert_eq!(tr("menu-file"), "File");
        assert_eq!(
            tr_args("menu-ui-scale-step", &[("percent", "150".into())]),
            "150 %"
        );
    }

    #[test]
    fn counts_pick_plural_forms() {
        assert_eq!(
            tr_args("status-copied", &[("count", 1usize.into())]),
            "Copied 1 node"
        );
        assert_eq!(
            tr_args("status-copied", &[("count", 3usize.into())]),
            "Copied 3 nodes"
        );
    }

    #[test]
    fn a_missing_message_shows_its_id() {
        assert_eq!(tr("no-such-message"), "no-such-message");
        let _p = Showing::language(Language::Pseudo);
        assert_eq!(tr("no-such-message"), "no-such-message");
    }

    #[test]
    fn pseudo_accents_lengthens_and_brackets() {
        let _p = Showing::language(Language::Pseudo);
        let file = tr("menu-file");
        assert!(file.starts_with("[Fîľé"), "{file}");
        assert!(file.ends_with(']'));
        assert!(is_pseudo(&file));
        // Arguments aren't accented: they're content, not the catalog's.
        let step = tr_args("menu-ui-scale-step", &[("percent", "150".into())]);
        assert!(step.starts_with("[150 %"), "{step}");
        // About a third longer.
        let english = std::thread::spawn(|| tr("action-delete-reconnect"))
            .join()
            .unwrap();
        let pseudo = tr("action-delete-reconnect");
        let (e, p) = (english.chars().count(), pseudo.chars().count());
        assert!(
            p * 100 >= e * 133 && p * 100 <= e * 140,
            "{english} → {pseudo}"
        );
        assert_eq!(pseudo_char('a'), 'å');
        assert!(!is_pseudo("File"));
    }

    #[test]
    fn languages_are_per_thread() {
        let _p = Showing::language(Language::Pseudo);
        assert!(is_pseudo(&tr("menu-file")));
        let other = std::thread::spawn(|| tr("menu-file")).join().unwrap();
        assert_eq!(other, "File");
    }

    #[test]
    fn system_locales_map_to_languages() {
        assert_eq!(Language::for_locale("en-US"), Some(Language::English));
        assert_eq!(Language::for_locale("en_GB.UTF-8"), Some(Language::English));
        assert_eq!(Language::for_locale("EN"), Some(Language::English));
        assert_eq!(Language::for_locale("de-DE"), None);
        assert_eq!(Language::for_locale(""), None);
        // Whatever the machine says, the result is a language the editor has.
        let _ = LanguageChoice::System.resolve();
        assert_eq!(LanguageChoice::Pseudo.resolve(), Language::Pseudo);
    }
}
