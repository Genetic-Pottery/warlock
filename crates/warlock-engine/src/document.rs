use std::collections::BTreeMap;
use std::fmt;
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use crate::agent::{File, Request};

pub const ENTRY_CHARS: usize = 280;

pub const ENTRY_MINIMUM: usize = 20;

pub const PURPOSE_CHARS: usize = 700;

pub const LIST_CAP: usize = 12;

pub const DECLARED_SHOWN: usize = 8;

// Two, not three: the second pass is the first one with its own defects listed
// at the top of the request, and a model shown that either fixes the slot or
// cannot. A third buys a third failure at full price.
pub const ATTEMPTS: usize = 2;

// No date in here, though every instinct says to put one: `granted_at` in
// `.warlock/pacts.toml` already records when the document was granted, and a
// date in the stamp would make a re-pact that changed no prose a diff.
pub const STAMP: &str = "<!-- warlock -->\n\
> Written by a model pass over this directory alone, to be read before its \
source and to say which source to read. A map, not a specification: check \
anything you are about to rely on against the files themselves, and where this \
document and the code disagree, the code is right.\n";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fill {
    #[serde(default)]
    pub purpose: String,
    #[serde(default)]
    pub files: BTreeMap<String, String>,
    #[serde(default)]
    pub directories: BTreeMap<String, String>,
    #[serde(default)]
    pub structure: Vec<String>,
    #[serde(default)]
    pub rules: Vec<String>,
    #[serde(default)]
    pub lookups: Vec<Lookup>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lookup {
    #[serde(rename = "for", default)]
    pub topic: String,
    #[serde(default)]
    pub open: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Described {
    pub declared: BTreeMap<String, Vec<String>>,
}

impl Fill {
    #[must_use]
    pub fn stub(request: &Request) -> Self {
        let expected = Expected::of(request);
        Self {
            purpose: format!(
                "A stand-in description of a directory of {} files and {} subdirectories, \
                 written by a test double that read none of them.",
                expected.files.len(),
                expected.directories.len()
            ),
            files: expected
                .asked()
                .map(|path| {
                    (
                        path.to_owned(),
                        "a stand-in entry, filled by a test double".to_owned(),
                    )
                })
                .collect(),
            directories: expected
                .directories
                .keys()
                .map(|name| {
                    (
                        (*name).to_owned(),
                        "a stand-in entry, filled by a test double".to_owned(),
                    )
                })
                .collect(),
            structure: Vec::new(),
            rules: Vec::new(),
            lookups: Vec::new(),
        }
    }

    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("a fill is plain strings and serialises")
    }
}

#[must_use]
pub fn stub_answer(request: &Request) -> String {
    const PROSE: &str = "A stand-in account of some contents, written by a test double that read \
                         none of them: records and pins, listed one after another with no code \
                         among them.";
    if request.prompt().starts_with(PROMPT) {
        Fill::stub(request).to_json()
    } else {
        PROSE.to_owned()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shown<'a> {
    Text(&'a str),
    NotText,
    Unsent,
}

#[derive(Debug)]
pub struct Expected<'a> {
    files: BTreeMap<&'a str, (u64, Shown<'a>)>,
    directories: BTreeMap<&'a str, &'a str>,
}

impl<'a> Expected<'a> {
    #[must_use]
    pub fn of(request: &'a Request) -> Self {
        let files = request
            .files()
            .iter()
            .map(|file| (file.path(), (file.size(), shown(file))))
            .collect();
        let directories = request
            .child_documents()
            .iter()
            .map(|child| (child.directory(), child.text()))
            .collect();
        Self { files, directories }
    }

    fn asked(&self) -> impl Iterator<Item = &'a str> + '_ {
        self.files
            .iter()
            .filter(|(_, (_, shown))| matches!(shown, Shown::Text(_)))
            .map(|(path, _)| *path)
    }

    fn checkable(&self, target: &str) -> Option<&'a str> {
        if let Some((_, shown)) = self.files.get(target) {
            return match shown {
                Shown::Text(text) => Some(text),
                Shown::NotText | Shown::Unsent => None,
            };
        }
        self.directories.get(target).copied()
    }

    fn mentions_tool(&self) -> bool {
        let in_files = self.files.values().any(|(_, shown)| match shown {
            Shown::Text(text) => names_tool(text),
            Shown::NotText | Shown::Unsent => false,
        });
        in_files
            || self
                .directories
                .values()
                .any(|text| names_tool(text.strip_prefix(STAMP).unwrap_or(text)))
    }

    fn holds(&self, target: &str) -> bool {
        self.files.contains_key(target) || self.directories.contains_key(target)
    }
}

fn names_tool(text: &str) -> bool {
    text.to_ascii_lowercase().contains("warlock")
}

fn shown(file: &File) -> Shown<'_> {
    if let Some(kept) = file.kept() {
        return Shown::Text(kept);
    }
    match file.bytes().map(std::str::from_utf8) {
        Some(Ok(text)) => Shown::Text(text),
        Some(Err(_)) => Shown::NotText,
        None => Shown::Unsent,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Defect {
    NotJson {
        detail: String,
    },
    Missing {
        field: String,
    },
    Empty {
        field: String,
    },
    Multiline {
        field: String,
    },
    TooShort {
        field: String,
        chars: usize,
        minimum: usize,
    },
    TooLong {
        field: String,
        chars: usize,
        cap: usize,
    },
    TooMany {
        field: String,
        count: usize,
        cap: usize,
    },
    UnknownTarget {
        field: String,
        open: String,
    },
    ToolNamed {
        field: String,
    },
    UnverifiedSymbol {
        field: String,
        symbol: String,
        open: String,
    },
}

impl fmt::Display for Defect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotJson { detail } => write!(f, "the answer is not a JSON object: {detail}"),
            Self::Missing { field } => write!(f, "{field} is missing"),
            Self::Empty { field } => write!(f, "{field} is empty"),
            Self::Multiline { field } => write!(f, "{field} runs to more than one line"),
            Self::TooShort {
                field,
                chars,
                minimum,
            } => write!(
                f,
                "{field} is {chars} characters, under the {minimum} it has to reach"
            ),
            Self::TooLong { field, chars, cap } => {
                write!(f, "{field} is {chars} characters, over the cap of {cap}")
            }
            Self::TooMany { field, count, cap } => {
                write!(f, "{field} has {count} entries, over the cap of {cap}")
            }
            Self::ToolNamed { field } => write!(
                f,
                "{field} names warlock, which is the tool writing this document and not \
                 something the files mention"
            ),
            Self::UnknownTarget { field, open } => write!(
                f,
                "{field} opens `{open}`, which is not a file or subdirectory here"
            ),
            Self::UnverifiedSymbol {
                field,
                symbol,
                open,
            } => write!(
                f,
                "{field} names `{symbol}`, which does not occur in what was shown of `{open}`"
            ),
        }
    }
}

impl std::error::Error for Defect {}

pub(crate) const PROMPT: &str = "\
Fill in the JSON object at the end of these instructions, describing the \
directory whose contents follow them, and output the filled object and nothing \
else.

You are filling in the WARLOCK.md for one directory of a codebase. It is read \
by a model, not a person, before any source file is opened, and its one job is \
routing: to say what is here and which file or subdirectory to open for a given \
question. Warlock is the tool that lays the document out from your answer; it \
is not the project being described, and its name belongs in no value unless \
the files themselves use it. Write only the values.

\"purpose\": one to three sentences. What this directory is and what it does, \
in the words a question about it would use. Name the product or subsystem it \
belongs to when the files say what that is.

\"files\": one line per key. What the file is and what it holds, naming the \
types, functions or constants a reader would come to it for, spelt as the file \
spells them. A file that is small, generated, or a re-export gets a line saying \
so. Every key is a file you were shown; add none and remove none.

\"directories\": one line per key. What is under it and the kind of question \
that should send a reader there. Write it from the subdirectory's own \
WARLOCK.md, which follows below, and do not restate that document's contents.

\"structure\": how the files here fit together, one fact per line: what calls \
what, in what order, which way a dependency runs. Only what the files you were \
shown show. An empty list is fine.

\"rules\": constraints this directory's own files state as rules, one per line: \
an invariant a comment asserts, a check the code makes, a setting a manifest \
pins. Not something inferred, and nothing taken from a subdirectory's document. \
An empty list is fine.

\"lookups\": routes, each {\"for\": ..., \"open\": ..., \"symbol\": ...}. \"for\" \
is a question or topic a reader might arrive with, in plain words. \"open\" is \
exactly one key of \"files\" or \"directories\". \"symbol\" is optional and \
must be a name that occurs verbatim in that file, or in that subdirectory's \
WARLOCK.md. Prefer the routes a reader could not guess from the file names.

Every value is one line. Write about the directory in its own voice: no first \
person, nothing about this request or about what you were or were not shown, \
and no guesses about files whose text is not here. Where a file and any \
document disagree, the file is right.

An answer is turned down and asked for again when a key is missing or \
invented, a value is empty or spans lines or runs long, a list is over its cap, \
or a lookup names a file or symbol that is not here.";

#[must_use]
pub fn instructions(expected: &Expected<'_>, rejected: &[Defect]) -> String {
    let mut text = PROMPT.to_owned();
    if !rejected.is_empty() {
        text.push_str(
            "\n\nA previous answer to exactly this request was turned down. Do not repeat \
             these defects:",
        );
        for defect in rejected {
            let _ = write!(text, "\n- {defect}");
        }
    }
    let _ = write!(
        text,
        "\n\nCaps: {ENTRY_MINIMUM} to {ENTRY_CHARS} characters per value, {PURPOSE_CHARS} for the \
         purpose, {LIST_CAP} entries per list.\n\n\
         Return exactly this object with every empty string filled in and the lists \
         populated, as JSON, with no code fence and nothing before or after it:\n\n{}",
        skeleton(expected)
    );
    text
}

#[must_use]
pub fn skeleton(expected: &Expected<'_>) -> String {
    let blank = Fill {
        purpose: String::new(),
        files: expected
            .asked()
            .map(|path| (path.to_owned(), String::new()))
            .collect(),
        directories: expected
            .directories
            .keys()
            .map(|name| ((*name).to_owned(), String::new()))
            .collect(),
        structure: Vec::new(),
        rules: Vec::new(),
        lookups: Vec::new(),
    };
    blank.to_json()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Accepted {
    Filled(Fill),
    Defective {
        fill: Fill,
        defects: Vec<Defect>,
    },
    Unparsed(Defect),
}

impl Accepted {
    pub fn settled(self) -> Result<Fill, Vec<Defect>> {
        match self {
            Self::Filled(fill) => Ok(fill),
            Self::Defective { defects, .. } => Err(defects),
            Self::Unparsed(defect) => Err(vec![defect]),
        }
    }
}

#[must_use]
pub fn accept(
    carried: Option<(&Fill, &Repair)>,
    answer: &str,
    expected: &Expected<'_>,
) -> Accepted {
    let parsed = match parse(answer) {
        Ok(parsed) => parsed,
        Err(defect) => return Accepted::Unparsed(defect),
    };
    // Nothing carried is the same operation with nothing to write over: the
    // answer is its own base, and an empty repair changes no slot — it is here
    // for the entries it drops, which is what both roads share.
    let blank = Repair::default();
    let (previous, repair) = carried.unwrap_or((&parsed, &blank));

    let fill = repair.apply(previous, &parsed, expected);
    let defects = check(&fill, expected);
    if defects.is_empty() {
        Accepted::Filled(fill)
    } else {
        Accepted::Defective { fill, defects }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Repair {
    pub purpose: bool,
    pub files: Vec<String>,
    pub directories: Vec<String>,
    pub lists: Vec<String>,
}

impl Repair {
    #[must_use]
    pub fn of(defects: &[Defect]) -> Self {
        let mut repair = Self::default();
        for defect in defects {
            let field = match defect {
                // Nothing to ask: there is no slot, only an answer that was
                // not an object.
                Defect::NotJson { .. } => continue,
                Defect::Missing { field }
                | Defect::Empty { field }
                | Defect::Multiline { field }
                | Defect::TooShort { field, .. }
                | Defect::TooLong { field, .. }
                | Defect::TooMany { field, .. }
                | Defect::UnknownTarget { field, .. }
                | Defect::ToolNamed { field }
                | Defect::UnverifiedSymbol { field, .. } => field.as_str(),
            };
            repair.note(field);
        }
        repair
    }

    fn note(&mut self, field: &str) {
        let (slot, rest) = field.split_once('[').unwrap_or((field, ""));
        match slot {
            "purpose" => self.purpose = true,
            "structure" | "rules" | "lookups" => {
                if !self.lists.iter().any(|list| list == slot) {
                    self.lists.push(slot.to_owned());
                }
            }
            "files" | "directories" => {
                let key: String =
                    serde_json::from_str(rest.trim_end_matches(']')).unwrap_or_default();
                let list = if slot == "files" {
                    &mut self.files
                } else {
                    &mut self.directories
                };
                if !key.is_empty() && !list.contains(&key) {
                    list.push(key);
                }
            }
            _ => {}
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        !self.purpose
            && self.files.is_empty()
            && self.directories.is_empty()
            && self.lists.is_empty()
    }

    fn asks_list(&self, name: &str) -> bool {
        self.lists.iter().any(|list| list == name)
    }

    #[must_use]
    pub fn skeleton(&self) -> String {
        let mut object = serde_json::Map::new();
        if self.purpose {
            object.insert(
                "purpose".to_owned(),
                serde_json::Value::String(String::new()),
            );
        }
        for (name, keys) in [("files", &self.files), ("directories", &self.directories)] {
            if !keys.is_empty() {
                let entries = keys
                    .iter()
                    .map(|key| (key.clone(), serde_json::Value::String(String::new())))
                    .collect();
                object.insert(name.to_owned(), serde_json::Value::Object(entries));
            }
        }
        for name in ["structure", "rules", "lookups"] {
            if self.asks_list(name) {
                object.insert(name.to_owned(), serde_json::Value::Array(Vec::new()));
            }
        }
        serde_json::to_string_pretty(&serde_json::Value::Object(object))
            .expect("a map of strings and arrays serialises")
    }

    fn previously(&self, previous: &Fill) -> String {
        let patch = Fill {
            purpose: if self.purpose {
                previous.purpose.clone()
            } else {
                String::new()
            },
            files: previous
                .files
                .iter()
                .filter(|(key, _)| self.files.contains(key))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            directories: previous
                .directories
                .iter()
                .filter(|(key, _)| self.directories.contains(key))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            structure: if self.asks_list("structure") {
                previous.structure.clone()
            } else {
                Vec::new()
            },
            rules: if self.asks_list("rules") {
                previous.rules.clone()
            } else {
                Vec::new()
            },
            lookups: if self.asks_list("lookups") {
                previous.lookups.clone()
            } else {
                Vec::new()
            },
        };
        patch.to_json()
    }

    #[must_use]
    pub fn apply(&self, previous: &Fill, patch: &Fill, expected: &Expected<'_>) -> Fill {
        let mut fill = previous.clone();
        // A keyed slot the patch left out keeps what it had — and keeps its
        // defect, which the check reports honestly — rather than being blanked.
        if self.purpose && !patch.purpose.trim().is_empty() {
            fill.purpose.clone_from(&patch.purpose);
        }
        for key in &self.files {
            if let Some(value) = patch.files.get(key) {
                fill.files.insert(key.clone(), value.clone());
            }
        }
        for key in &self.directories {
            if let Some(value) = patch.directories.get(key) {
                fill.directories.insert(key.clone(), value.clone());
            }
        }
        if self.asks_list("structure") {
            fill.structure.clone_from(&patch.structure);
        }
        if self.asks_list("rules") {
            fill.rules.clone_from(&patch.rules);
        }
        if self.asks_list("lookups") {
            fill.lookups.clone_from(&patch.lookups);
        }
        let asked: Vec<&str> = expected.asked().collect();
        fill.files.retain(|key, _| asked.contains(&key.as_str()));
        fill.directories
            .retain(|key, _| expected.directories.contains_key(key.as_str()));
        fill
    }
}

#[must_use]
pub fn repair_instructions(
    expected: &Expected<'_>,
    previous: &Fill,
    defects: &[Defect],
    repair: &Repair,
) -> String {
    let _ = expected;
    let mut text = PROMPT.to_owned();
    text.push_str(
        "\n\nYour previous answer to exactly this request was turned down for the defects \
         below. Everything else in it is kept. Return a JSON object holding ONLY the slots in \
         the skeleton at the end, corrected: for \"files\" and \"directories\", the entries \
         named; for a list, the whole list again, corrected.",
    );
    text.push_str("\n\nThe defects:");
    for defect in defects {
        let _ = write!(text, "\n- {defect}");
    }
    let _ = write!(
        text,
        "\n\nWhat you wrote for those slots:\n\n{}\n\nCaps: {ENTRY_CHARS} characters per value, \
         {PURPOSE_CHARS} for the purpose, {LIST_CAP} entries per list.\n\n\
         Return exactly this object filled in, as JSON, with no code fence and nothing before \
         or after it:\n\n{}",
        repair.previously(previous),
        repair.skeleton()
    );
    text
}

fn parse(answer: &str) -> Result<Fill, Defect> {
    parse_object(answer)
}

fn parse_object<T: serde::de::DeserializeOwned>(answer: &str) -> Result<T, Defect> {
    let start = answer.find('{');
    let end = answer.rfind('}');
    let (Some(start), Some(end)) = (start, end) else {
        return Err(Defect::NotJson {
            detail: "no object found in the answer".to_owned(),
        });
    };
    if end < start {
        return Err(Defect::NotJson {
            detail: "no object found in the answer".to_owned(),
        });
    }
    serde_json::from_str(&answer[start..=end]).map_err(|error| Defect::NotJson {
        detail: error.to_string(),
    })
}

fn check(fill: &Fill, expected: &Expected<'_>) -> Vec<Defect> {
    let mut defects = Vec::new();

    line(
        "purpose",
        &fill.purpose,
        ENTRY_MINIMUM,
        PURPOSE_CHARS,
        &mut defects,
    );

    let asked: Vec<&str> = expected.asked().collect();
    keyed("files", &fill.files, &asked, &mut defects);
    let children: Vec<&str> = expected.directories.keys().copied().collect();
    keyed("directories", &fill.directories, &children, &mut defects);

    listed("structure", &fill.structure, &mut defects);
    listed("rules", &fill.rules, &mut defects);

    if fill.lookups.len() > LIST_CAP {
        defects.push(Defect::TooMany {
            field: "lookups".to_owned(),
            count: fill.lookups.len(),
            cap: LIST_CAP,
        });
    }
    for (index, lookup) in fill.lookups.iter().enumerate() {
        route(index, lookup, expected, &mut defects);
    }

    // Measured, not hypothetical: told it is filling in "the WARLOCK.md" and
    // that "warlock lays the document out", a pass over a crate that never
    // mentions warlock called it "a toy freshness ledger belonging to
    // Warlock". The instructions say the name is the tool's; this is the check
    // behind the sentence, and it stands down the moment the files use it.
    if !expected.mentions_tool() {
        for (field, value) in values(fill) {
            if names_tool(&value) {
                defects.push(Defect::ToolNamed { field });
            }
        }
    }

    defects
}

fn values(fill: &Fill) -> Vec<(String, String)> {
    let mut all = vec![("purpose".to_owned(), fill.purpose.clone())];
    all.extend(
        fill.files
            .iter()
            .map(|(k, v)| (format!("files[{k:?}]"), v.clone())),
    );
    all.extend(
        fill.directories
            .iter()
            .map(|(k, v)| (format!("directories[{k:?}]"), v.clone())),
    );
    for (name, list) in [("structure", &fill.structure), ("rules", &fill.rules)] {
        all.extend(
            list.iter()
                .enumerate()
                .map(|(i, v)| (format!("{name}[{i}]"), v.clone())),
        );
    }
    for (i, lookup) in fill.lookups.iter().enumerate() {
        all.push((format!("lookups[{i}].for"), lookup.topic.clone()));
    }
    all
}

fn line(field: &str, value: &str, minimum: usize, cap: usize, defects: &mut Vec<Defect>) {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        defects.push(Defect::Empty {
            field: field.to_owned(),
        });
        return;
    }
    if trimmed.contains(['\n', '\r']) {
        defects.push(Defect::Multiline {
            field: field.to_owned(),
        });
    }
    let chars = trimmed.chars().count();
    if chars < minimum {
        defects.push(Defect::TooShort {
            field: field.to_owned(),
            chars,
            minimum,
        });
    }
    if chars > cap {
        defects.push(Defect::TooLong {
            field: field.to_owned(),
            chars,
            cap,
        });
    }
}

fn keyed(name: &str, given: &BTreeMap<String, String>, wanted: &[&str], defects: &mut Vec<Defect>) {
    for key in wanted {
        if !given.contains_key(*key) {
            defects.push(Defect::Missing {
                field: format!("{name}[{key:?}]"),
            });
        }
    }
    for (key, value) in given {
        // An entry for something the request does not hold cannot reach here:
        // it is taken out by `Repair::apply` before the check, because no
        // answer could make it right and a pass spent asking for one is a pass
        // wasted. See [`accept`].
        debug_assert!(
            wanted.contains(&key.as_str()),
            "{name}[{key:?}] was never asked for"
        );
        let field = format!("{name}[{key:?}]");
        line(&field, value, ENTRY_MINIMUM, ENTRY_CHARS, defects);
    }
}

fn listed(name: &str, given: &[String], defects: &mut Vec<Defect>) {
    if given.len() > LIST_CAP {
        defects.push(Defect::TooMany {
            field: name.to_owned(),
            count: given.len(),
            cap: LIST_CAP,
        });
    }
    for (index, value) in given.iter().enumerate() {
        line(
            &format!("{name}[{index}]"),
            value,
            ENTRY_MINIMUM,
            ENTRY_CHARS,
            defects,
        );
    }
}

fn route(index: usize, lookup: &Lookup, expected: &Expected<'_>, defects: &mut Vec<Defect>) {
    line(
        &format!("lookups[{index}].for"),
        &lookup.topic,
        1,
        ENTRY_CHARS,
        defects,
    );
    let open = lookup.open.trim();
    if open.is_empty() {
        defects.push(Defect::Empty {
            field: format!("lookups[{index}].open"),
        });
        return;
    }
    if !expected.holds(open) {
        defects.push(Defect::UnknownTarget {
            field: format!("lookups[{index}].open"),
            open: open.to_owned(),
        });
        return;
    }
    let Some(symbol) = lookup.symbol.as_deref().map(str::trim) else {
        return;
    };
    let field = format!("lookups[{index}].symbol");
    if symbol.is_empty() {
        defects.push(Defect::Empty { field });
        return;
    }
    let verified = expected
        .checkable(open)
        .is_some_and(|text| text.contains(symbol));
    if !verified {
        defects.push(Defect::UnverifiedSymbol {
            field,
            symbol: symbol.to_owned(),
            open: open.to_owned(),
        });
    }
}

#[must_use]
pub fn render(name: &str, fill: &Fill, expected: &Expected<'_>, described: &Described) -> String {
    let mut text = format!("{STAMP}\n# {name}\n\n{}\n", fill.purpose.trim());

    if !expected.files.is_empty() {
        text.push_str("\n## Files\n\n");
        for (path, (size, shown)) in &expected.files {
            let entry = match shown {
                Shown::Text(_) => fill.files.get(*path).map_or("", |line| line.trim()),
                Shown::NotText => "not text; name and size only",
                Shown::Unsent => "not read by the pass; name and size only",
            };
            let _ = write!(text, "- `{path}` ({}) — {entry}", human(*size));
            if let Some(names) = described
                .declared
                .get(*path)
                .filter(|names| !names.is_empty())
            {
                let shown: Vec<String> = names
                    .iter()
                    .take(DECLARED_SHOWN)
                    .map(|name| format!("`{name}`"))
                    .collect();
                let _ = write!(text, " · declares {}", shown.join(", "));
                if names.len() > DECLARED_SHOWN {
                    let _ = write!(text, " (+{})", names.len() - DECLARED_SHOWN);
                }
            }
            text.push('\n');
        }
    }

    if !expected.directories.is_empty() {
        text.push_str("\n## Directories\n\n");
        for child in expected.directories.keys() {
            let entry = fill.directories.get(*child).map_or("", |line| line.trim());
            let _ = writeln!(text, "- `{child}/` — {entry}");
        }
    }

    list(&mut text, "Structure", &fill.structure);
    list(&mut text, "Rules", &fill.rules);

    if !fill.lookups.is_empty() {
        text.push_str("\n## Where to look\n\n");
        for lookup in &fill.lookups {
            let _ = write!(text, "- {} → `{}`", lookup.topic.trim(), lookup.open.trim());
            if let Some(symbol) = lookup.symbol.as_deref().map(str::trim) {
                let _ = write!(text, " `{symbol}`");
            }
            text.push('\n');
        }
    }

    text
}

fn list(text: &mut String, heading: &str, entries: &[String]) {
    if entries.is_empty() {
        return;
    }
    let _ = write!(text, "\n## {heading}\n\n");
    for entry in entries {
        let _ = writeln!(text, "- {}", entry.trim());
    }
}

fn human(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * KB;
    if bytes < KB {
        return format!("{bytes} B");
    }
    let (unit, scale) = if bytes < MB { ("KB", KB) } else { ("MB", MB) };
    let tenths = bytes * 10 / scale;
    format!("{}.{} {unit}", tenths / 10, tenths % 10)
}

#[cfg(test)]
mod tests {
    use super::{
        ATTEMPTS, Accepted, Defect, Described, ENTRY_CHARS, ENTRY_MINIMUM, Expected, Fill,
        LIST_CAP, Lookup, PROMPT, PURPOSE_CHARS, Repair, STAMP, accept, human, instructions,
        render, repair_instructions, skeleton, stub_answer,
    };
    use std::collections::BTreeMap;

    use crate::agent::{ChildDocument, File, Request};

    fn request() -> Request {
        Request::new("describe", "/repo/crates/engine")
            .with_files([
                File::present("lib.rs", *b"pub mod pact;\npub fn subtree_hash() {}\n"),
                File::present("Cargo.toml", *b"[package]\nname = \"engine\"\n"),
                File::elided(
                    "app.rs",
                    900_000,
                    "pub fn draw() {}\n… 40000 lines elided …\n",
                ),
                File::omitted("Cargo.lock", 4_200_000),
                File::present("logo.png", vec![0x89, b'P', b'N', b'G', 0xff]),
            ])
            .with_child_documents([ChildDocument::new(
                "src",
                "# src\n\n## Where to look\n\n- hashing → `hash.rs` `subtree_hash`\n",
            )])
    }

    fn good() -> Fill {
        let mut fill = Fill::stub(&request());
        fill.purpose = "The engine crate: pacts, hashes and the manifest.".to_owned();
        fill.structure = vec!["`lib.rs` re-exports `pact` for the crate.".to_owned()];
        fill.rules = vec!["No `unsafe`, per the workspace lints.".to_owned()];
        fill.lookups = vec![
            Lookup {
                topic: "how a subtree is hashed".to_owned(),
                open: "lib.rs".to_owned(),
                symbol: Some("subtree_hash".to_owned()),
            },
            Lookup {
                topic: "the code itself".to_owned(),
                open: "src".to_owned(),
                symbol: None,
            },
        ];
        fill
    }

    fn defects(fill: &Fill) -> Vec<Defect> {
        let request = request();
        accept(None, &fill.to_json(), &Expected::of(&request))
            .settled()
            .err()
            .unwrap_or_default()
    }

    #[test]
    fn the_slots_are_the_files_shown_and_the_children_handed_over() {
        let request = request();
        let text = skeleton(&Expected::of(&request));
        let blank: Fill = serde_json::from_str(&text).expect("the skeleton is a fill");
        let keys: Vec<&str> = blank.files.keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            ["Cargo.toml", "app.rs", "lib.rs"],
            "a file whose text was shown, whole or reduced; not the over-cap or non-text ones",
        );
        let children: Vec<&str> = blank.directories.keys().map(String::as_str).collect();
        assert_eq!(children, ["src"]);
        assert!(blank.files.values().all(String::is_empty));
        assert!(blank.purpose.is_empty());
    }

    #[test]
    fn a_stub_is_accepted_for_any_request() {
        // As a pass sees it: the prompt a directory pass runs under is the
        // one `instructions` builds, and that is what tells a stand-in which
        // shape to answer in.
        let request = request();
        let asked = request
            .clone()
            .with_prompt(instructions(&Expected::of(&request), &[]));
        let fill = accept(None, &stub_answer(&asked), &Expected::of(&asked))
            .settled()
            .expect("accepted");
        assert_eq!(fill.files.len(), 3);

        let bare = Request::new("x", "/repo/empty");
        let bare = bare
            .clone()
            .with_prompt(instructions(&Expected::of(&bare), &[]));
        accept(None, &stub_answer(&bare), &Expected::of(&bare))
            .settled()
            .expect("accepted too");

        // Anything that is not a directory pass gets plain prose.
        assert!(!stub_answer(&request).trim_start().starts_with('{'));
    }

    #[test]
    fn a_good_fill_is_accepted() {
        assert_eq!(defects(&good()), []);
    }

    #[test]
    fn an_answer_wrapped_in_a_fence_or_a_sentence_is_still_read() {
        let request = request();
        let expected = Expected::of(&request);
        let json = good().to_json();
        for wrapped in [
            format!("```json\n{json}\n```"),
            format!("Here is the object:\n\n{json}\n\nLet me know if you need more."),
        ] {
            accept(None, &wrapped, &expected)
                .settled()
                .expect("read from the outermost braces");
        }
    }

    #[test]
    fn an_answer_that_is_not_an_object_is_one_defect() {
        let request = request();
        let expected = Expected::of(&request);
        for answer in [
            "",
            "no.",
            "# engine\n\nProse about the directory.",
            "[1, 2]",
            "{",
        ] {
            let outcome = accept(None, answer, &expected);
            assert!(
                matches!(outcome, Accepted::Unparsed(Defect::NotJson { .. })),
                "{answer:?}: {outcome:?}"
            );
            let found = outcome.settled().expect_err("turned down");
            assert_eq!(found.len(), 1, "{answer:?}: {found:?}");
        }
    }

    #[test]
    fn a_missing_key_is_named_and_an_invented_one_is_dropped() {
        let mut fill = good();
        fill.files.remove("lib.rs");
        fill.files
            .insert("main.rs".to_owned(), "invented, and not here".to_owned());

        // The slot that is missing is the one a second pass can do something
        // about, so it is the only one reported.
        assert_eq!(
            defects(&fill),
            [Defect::Missing {
                field: "files[\"lib.rs\"]".to_owned()
            }]
        );
    }

    #[test]
    fn a_file_the_pass_was_not_shown_is_not_a_slot_it_may_fill() {
        let request = request();
        let mut fill = good();
        fill.files.insert(
            "Cargo.lock".to_owned(),
            "the lockfile, presumably".to_owned(),
        );

        // Taken out rather than turned down: no answer could make an entry for
        // a file that is not here right, so it costs no pass and the rest of
        // the answer stands.
        let accepted = accept(None, &fill.to_json(), &Expected::of(&request))
            .settled()
            .expect("the invented entry is dropped, not refused");
        assert!(!accepted.files.contains_key("Cargo.lock"));
        assert_eq!(accepted.files.len(), 3);
    }

    #[test]
    fn a_bare_word_is_a_skipped_slot_and_not_an_entry() {
        // Measured: a pass answered `"duplicate"` for `clock.rs`, one word with
        // no fact in it, and every check of the day passed.
        let mut fill = good();
        fill.files
            .insert("lib.rs".to_owned(), "duplicate".to_owned());
        assert_eq!(
            defects(&fill),
            [Defect::TooShort {
                field: "files[\"lib.rs\"]".to_owned(),
                chars: 9,
                minimum: ENTRY_MINIMUM
            }]
        );
        // A route's question may be short: "hashing" is a fine topic.
        let mut fill = good();
        fill.lookups[0].topic = "hashing".to_owned();
        assert_eq!(defects(&fill), []);
    }

    #[test]
    fn every_value_is_one_line_under_its_cap() {
        let mut fill = good();
        fill.purpose = "   ".to_owned();
        fill.files.insert(
            "lib.rs".to_owned(),
            "two\nlines, and long enough besides".to_owned(),
        );
        fill.structure = vec!["x".repeat(ENTRY_CHARS + 1)];
        let found = defects(&fill);
        assert!(found.contains(&Defect::Empty {
            field: "purpose".to_owned()
        }));
        assert!(found.contains(&Defect::Multiline {
            field: "files[\"lib.rs\"]".to_owned()
        }));
        assert!(found.contains(&Defect::TooLong {
            field: "structure[0]".to_owned(),
            chars: ENTRY_CHARS + 1,
            cap: ENTRY_CHARS
        }));
        assert_eq!(found.len(), 3, "{found:?}");
    }

    #[test]
    fn the_purpose_has_its_own_longer_cap() {
        let mut fill = good();
        fill.purpose = "p".repeat(PURPOSE_CHARS);
        assert_eq!(defects(&fill), []);
        fill.purpose.push('p');
        assert_eq!(defects(&fill).len(), 1);
    }

    #[test]
    fn a_list_over_the_cap_is_a_defect_and_so_is_each_bad_entry_in_it() {
        let mut fill = good();
        fill.rules = vec!["a rule long enough to count".to_owned(); LIST_CAP + 1];
        fill.rules[3] = String::new();
        assert_eq!(
            defects(&fill),
            [
                Defect::TooMany {
                    field: "rules".to_owned(),
                    count: LIST_CAP + 1,
                    cap: LIST_CAP
                },
                Defect::Empty {
                    field: "rules[3]".to_owned()
                }
            ]
        );
    }

    #[test]
    fn a_lookup_must_open_something_here_and_a_symbol_must_be_in_it() {
        let mut fill = good();
        fill.lookups = vec![
            Lookup {
                topic: "somewhere else".to_owned(),
                open: "../tui/src/app.rs".to_owned(),
                symbol: None,
            },
            Lookup {
                topic: "a name that is not there".to_owned(),
                open: "lib.rs".to_owned(),
                symbol: Some("load_tree".to_owned()),
            },
            Lookup {
                topic: "a name in a file nobody read".to_owned(),
                open: "Cargo.lock".to_owned(),
                symbol: Some("serde".to_owned()),
            },
            Lookup {
                topic: "a name in a reduced file's surviving lines".to_owned(),
                open: "app.rs".to_owned(),
                symbol: Some("draw".to_owned()),
            },
            Lookup {
                topic: "a name the child's document states".to_owned(),
                open: "src".to_owned(),
                symbol: Some("subtree_hash".to_owned()),
            },
        ];
        assert_eq!(
            defects(&fill),
            [
                Defect::UnknownTarget {
                    field: "lookups[0].open".to_owned(),
                    open: "../tui/src/app.rs".to_owned()
                },
                Defect::UnverifiedSymbol {
                    field: "lookups[1].symbol".to_owned(),
                    symbol: "load_tree".to_owned(),
                    open: "lib.rs".to_owned()
                },
                Defect::UnverifiedSymbol {
                    field: "lookups[2].symbol".to_owned(),
                    symbol: "serde".to_owned(),
                    open: "Cargo.lock".to_owned()
                },
            ],
            "the last two verify against a reduced file's own lines and a child document",
        );
    }

    #[test]
    fn naming_the_tool_is_a_defect_unless_the_files_name_it() {
        let mut fill = good();
        fill.purpose = "A toy ledger belonging to Warlock.".to_owned();
        fill.structure = vec!["warlock-style grants, one per module".to_owned()];
        assert_eq!(
            defects(&fill),
            [
                Defect::ToolNamed {
                    field: "purpose".to_owned()
                },
                Defect::ToolNamed {
                    field: "structure[0]".to_owned()
                },
            ]
        );

        // A child's document always carries the stamp, and the stamp is not the
        // files naming the tool.
        let parent = Request::new("describe", "/repo")
            .with_child_documents([ChildDocument::new("src", format!("{STAMP}\n# src\n"))]);
        let mut fill = Fill::stub(&parent);
        fill.purpose = "Warlock's source, in one crate.".to_owned();
        assert!(matches!(
            accept(None, &fill.to_json(), &Expected::of(&parent))
                .settled()
                .expect_err("refused")
                .as_slice(),
            [Defect::ToolNamed { .. }]
        ));

        // Warlock's own repository names itself, and may say so.
        let own = Request::new("describe", "/repo")
            .with_files([File::present("lib.rs", *b"//! Core engine for warlock.\n")]);
        let mut fill = Fill::stub(&own);
        fill.purpose = "Warlock's engine crate, in one line.".to_owned();
        accept(None, &fill.to_json(), &Expected::of(&own))
            .settled()
            .expect("accepted");
    }

    #[test]
    fn the_document_is_laid_out_by_warlock_and_not_by_the_pass() {
        let request = request();
        let expected = Expected::of(&request);
        let described = Described {
            declared: [
                (
                    "lib.rs".to_owned(),
                    vec!["pact".to_owned(), "subtree_hash".to_owned()],
                ),
                (
                    "app.rs".to_owned(),
                    (0..10).map(|i| format!("draw{i}")).collect(),
                ),
            ]
            .into_iter()
            .collect(),
        };
        let text = render("engine", &good(), &expected, &described);
        assert!(text.starts_with(STAMP), "{text}");
        assert_eq!(
            text.strip_prefix(STAMP).expect("stamped"),
            "\n# engine\n\n\
             The engine crate: pacts, hashes and the manifest.\n\
             \n## Files\n\n\
             - `Cargo.lock` (4.0 MB) — not read by the pass; name and size only\n\
             - `Cargo.toml` (26 B) — a stand-in entry, filled by a test double\n\
             - `app.rs` (878.9 KB) — a stand-in entry, filled by a test double · declares `draw0`, `draw1`, `draw2`, `draw3`, `draw4`, `draw5`, `draw6`, `draw7` (+2)\n\
             - `lib.rs` (39 B) — a stand-in entry, filled by a test double · declares `pact`, `subtree_hash`\n\
             - `logo.png` (5 B) — not text; name and size only\n\
             \n## Directories\n\n\
             - `src/` — a stand-in entry, filled by a test double\n\
             \n## Structure\n\n\
             - `lib.rs` re-exports `pact` for the crate.\n\
             \n## Rules\n\n\
             - No `unsafe`, per the workspace lints.\n\
             \n## Where to look\n\n\
             - how a subtree is hashed → `lib.rs` `subtree_hash`\n\
             - the code itself → `src`\n"
        );
    }

    #[test]
    fn an_empty_list_is_no_section_at_all() {
        let bare = Request::new("describe", "/repo/empty");
        let mut fill = Fill::stub(&bare);
        fill.purpose = "Nothing here yet, but the directory exists.".to_owned();
        let text = render("empty", &fill, &Expected::of(&bare), &Described::default());
        assert_eq!(
            text.strip_prefix(STAMP),
            Some("\n# empty\n\nNothing here yet, but the directory exists.\n")
        );
        for heading in [
            "## Files",
            "## Directories",
            "## Structure",
            "## Rules",
            "## Where",
        ] {
            assert!(!text.contains(heading), "{text}");
        }
    }

    #[test]
    fn values_are_trimmed_on_the_way_out_and_not_on_the_way_in() {
        let request = request();
        let mut fill = good();
        fill.files.insert(
            "lib.rs".to_owned(),
            "  padded, but long enough  ".to_owned(),
        );
        assert_eq!(defects(&fill), [], "padding is not a defect");
        let text = render(
            "engine",
            &fill,
            &Expected::of(&request),
            &Described::default(),
        );
        assert!(
            text.contains("- `lib.rs` (39 B) — padded, but long enough\n"),
            "{text}"
        );
    }

    #[test]
    fn the_instructions_carry_the_skeleton_and_any_rejection() {
        let request = request();
        let expected = Expected::of(&request);
        let first = instructions(&expected, &[]);
        assert!(first.starts_with(PROMPT));
        assert!(
            first.ends_with(&skeleton(&expected)),
            "the skeleton is last"
        );
        assert!(!first.contains("turned down. Do not repeat"));
        assert!(first.contains(&format!(
            "{ENTRY_MINIMUM} to {ENTRY_CHARS} characters per value"
        )));

        let second = instructions(
            &expected,
            &[Defect::Missing {
                field: "files[\"lib.rs\"]".to_owned(),
            }],
        );
        assert!(
            second.contains(
                "turned down. Do not repeat these defects:\n- files[\"lib.rs\"] is missing"
            )
        );
        assert!(second.ends_with(&skeleton(&expected)));
    }

    #[test]
    fn the_prompt_says_warlock_is_the_tool_and_not_the_project() {
        // Measured on a scratch crate that never mentions warlock: two of its
        // three documents called it "a toy freshness ledger belonging to
        // Warlock", because the instructions name warlock and ask for the
        // product the directory belongs to.
        assert!(
            PROMPT.contains("it is not the project being described"),
            "{PROMPT}"
        );
        assert!(
            PROMPT.contains("its name belongs in no value unless the files themselves use it"),
            "{PROMPT}"
        );
    }

    #[test]
    fn a_repair_asks_again_for_exactly_the_slots_at_fault() {
        let found = [
            Defect::Missing {
                field: "files[\"tree.rs\"]".to_owned(),
            },
            Defect::TooLong {
                field: "files[\"lib.rs\"]".to_owned(),
                chars: 300,
                cap: ENTRY_CHARS,
            },
            Defect::UnverifiedSymbol {
                field: "lookups[2].symbol".to_owned(),
                symbol: "x".to_owned(),
                open: "lib.rs".to_owned(),
            },
        ];
        let repair = Repair::of(&found);
        assert_eq!(
            repair,
            Repair {
                purpose: false,
                files: vec!["tree.rs".to_owned(), "lib.rs".to_owned()],
                directories: Vec::new(),
                lists: vec!["lookups".to_owned()],
            },
            "a list is re-asked whole"
        );
        assert_eq!(
            repair.skeleton(),
            "{\n  \"files\": {\n    \"tree.rs\": \"\",\n    \"lib.rs\": \"\"\n  },\n  \"lookups\": []\n}"
        );
        assert!(
            Repair::of(&[Defect::NotJson {
                detail: "expected value".to_owned()
            }])
            .is_empty(),
            "an answer that is not an object names no slot to ask again for"
        );
    }

    #[test]
    fn a_patch_is_written_over_the_slots_at_fault_and_nothing_else() {
        let request = request();
        let expected = Expected::of(&request);
        let mut previous = good();
        previous
            .files
            .insert("ghost.rs".to_owned(), "not here at all".to_owned());
        previous.files.remove("lib.rs");
        let repair = Repair {
            files: vec!["lib.rs".to_owned()],
            lists: vec!["lookups".to_owned()],
            ..Repair::default()
        };
        let patch = Fill {
            purpose: "ignored: not asked for".to_owned(),
            files: [(
                "lib.rs".to_owned(),
                "the crate root, repaired by a patch".to_owned(),
            )]
            .into_iter()
            .collect(),
            ..Fill::default()
        };
        let mended = repair.apply(&previous, &patch, &expected);
        assert_eq!(
            mended.purpose, previous.purpose,
            "a slot not asked for keeps its value"
        );
        assert_eq!(
            mended.files["lib.rs"],
            "the crate root, repaired by a patch"
        );
        assert!(
            !mended.files.contains_key("ghost.rs"),
            "an entry for a file that is not here goes"
        );
        assert!(
            mended.lookups.is_empty(),
            "a list asked for again is replaced whole"
        );
        assert_eq!(mended.rules, previous.rules);
        assert_eq!(
            accept(None, &mended.to_json(), &expected),
            Accepted::Filled(mended.clone())
        );
    }

    #[test]
    fn the_repair_instructions_show_what_was_written_and_ask_for_the_slots_alone() {
        let request = request();
        let expected = Expected::of(&request);
        let previous = good();
        let found = [Defect::TooLong {
            field: "files[\"lib.rs\"]".to_owned(),
            chars: 300,
            cap: ENTRY_CHARS,
        }];
        let repair = Repair::of(&found);
        let text = repair_instructions(&expected, &previous, &found, &repair);
        assert!(text.starts_with(PROMPT));
        assert!(text.contains("holding ONLY the slots"));
        assert!(text.contains("- files[\"lib.rs\"] is 300 characters"));
        assert!(
            text.contains("\"lib.rs\": \"a stand-in entry, filled by a test double\""),
            "what it wrote: {text}"
        );
        assert!(text.ends_with(&repair.skeleton()));
        const {
            assert!(ATTEMPTS >= 2, "there is a second attempt to show these to");
        }
    }

    #[test]
    fn every_defect_reads_as_one_line_naming_its_slot() {
        let all = [
            Defect::NotJson {
                detail: "expected value at line 1".to_owned(),
            },
            Defect::Missing {
                field: "files[\"a\"]".to_owned(),
            },
            Defect::Empty {
                field: "purpose".to_owned(),
            },
            Defect::Multiline {
                field: "rules[0]".to_owned(),
            },
            Defect::TooShort {
                field: "files[\"c\"]".to_owned(),
                chars: 3,
                minimum: ENTRY_MINIMUM,
            },
            Defect::TooLong {
                field: "structure[1]".to_owned(),
                chars: 300,
                cap: ENTRY_CHARS,
            },
            Defect::TooMany {
                field: "lookups".to_owned(),
                count: 20,
                cap: LIST_CAP,
            },
            Defect::UnknownTarget {
                field: "lookups[0].open".to_owned(),
                open: "x".to_owned(),
            },
            Defect::ToolNamed {
                field: "purpose".to_owned(),
            },
            Defect::UnverifiedSymbol {
                field: "lookups[0].symbol".to_owned(),
                symbol: "y".to_owned(),
                open: "x".to_owned(),
            },
        ];
        for defect in all {
            let text = defect.to_string();
            assert!(!text.contains('\n'), "{text}");
            assert!(!text.is_empty());
        }
    }

    #[test]
    fn sizes_read_the_way_a_person_reads_them() {
        assert_eq!(human(0), "0 B");
        assert_eq!(human(1023), "1023 B");
        assert_eq!(human(1024), "1.0 KB");
        assert_eq!(human(22_938), "22.4 KB");
        assert_eq!(human(1024 * 1024), "1.0 MB");
        assert_eq!(human(4_200_000), "4.0 MB");
    }

    #[test]
    fn a_described_record_is_only_what_warlock_measured() {
        let described = Described {
            declared: [("a.rs".to_owned(), vec!["one".to_owned()])]
                .into_iter()
                .collect(),
        };
        assert_eq!(described.declared["a.rs"], ["one"]);
        assert_eq!(Described::default().declared, BTreeMap::new());
    }
}
