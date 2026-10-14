//! Typed shape of `master_cv.yaml` and of one application file.
//!
//! `templates/cv.typ` reads the same YAML, so a field renamed here must be renamed there.
//! Every struct denies unknown fields: a misspelled key is a parse error, not a silent no-op.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// A CV language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    /// English.
    En,
    /// German.
    De,
}

impl Lang {
    /// Every language a CV builds in, in build order.
    pub const ALL: [Lang; 2] = [Lang::En, Lang::De];

    /// The code used in file names and on the command line.
    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::De => "de",
        }
    }

    /// Parse `en` or `de`.
    ///
    /// # Errors
    /// Any other string.
    pub fn parse(code: &str) -> Result<Lang, String> {
        match code {
            "en" => Ok(Lang::En),
            "de" => Ok(Lang::De),
            other => Err(format!("language must be en or de, not '{other}'")),
        }
    }
}

/// Text in both languages. A missing `de` renders the `en` text.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Localized {
    /// English text.
    pub en: String,
    /// German text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub de: Option<String>,
}

impl Localized {
    /// The text for `lang`, falling back to English.
    pub fn get(&self, lang: Lang) -> &str {
        match (lang, &self.de) {
            (Lang::De, Some(de)) => de,
            _ => &self.en,
        }
    }
}

/// One string shared by both languages, or `{en, de}` where they differ.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Text {
    /// Same text in every language, such as a company name.
    Plain(String),
    /// Text that differs by language.
    Localized(Localized),
}

impl Text {
    /// The text for `lang`.
    pub fn get(&self, lang: Lang) -> &str {
        match self {
            Text::Plain(s) => s,
            Text::Localized(l) => l.get(lang),
        }
    }
}

/// How long an item survives a one-page build: 1 is kept longest, 3 is dropped first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Priority {
    /// The same level in every profile.
    Level(u8),
    /// A level per profile, with `default` for the rest.
    PerProfile(IndexMap<String, u8>),
}

impl Priority {
    /// The level that applies in `profile`.
    pub fn level(&self, profile: &str) -> u8 {
        match self {
            Priority::Level(n) => *n,
            Priority::PerProfile(map) => map
                .get(profile)
                .or_else(|| map.get("default"))
                .copied()
                .unwrap_or(1),
        }
    }

    fn levels(&self) -> Vec<u8> {
        match self {
            Priority::Level(n) => vec![*n],
            Priority::PerProfile(map) => map.values().copied().collect(),
        }
    }
}

/// Selection fields every filterable item carries.
pub trait Item {
    /// Stable name an application can `pin` or `hide`.
    fn id(&self) -> Option<&str>;
    /// Profiles that include the item. Empty means every profile.
    fn profiles(&self) -> &[String];
    /// Survival level in a one-page build. Absent means 1.
    fn priority(&self) -> Option<&Priority>;
}

macro_rules! item {
    ($t:ty) => {
        impl Item for $t {
            fn id(&self) -> Option<&str> {
                self.id.as_deref()
            }
            fn profiles(&self) -> &[String] {
                &self.profiles
            }
            fn priority(&self) -> Option<&Priority> {
                self.priority.as_ref()
            }
        }
    };
}

/// Localized text that is also a filterable item: an education detail or a section intro.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalizedItem {
    /// English text.
    pub en: String,
    /// German text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub de: Option<String>,
    /// See [`Item::id`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// See [`Item::profiles`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub profiles: Vec<String>,
    /// See [`Item::priority`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<Priority>,
    /// Where the fact comes from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}
item!(LocalizedItem);

/// Contact line under the name.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Contact {
    /// Email address.
    pub email: String,
    /// Phone number, shown only when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phone: Option<String>,
    /// City and country.
    pub location: String,
    /// LinkedIn URL without scheme.
    pub linkedin: String,
    /// GitHub URL without scheme.
    pub github: String,
}

/// LinkedIn's own headline and About text. The PDF does not render them.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinkedinText {
    /// Headline under the name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headline: Option<Localized>,
    /// About section.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub about: Option<Localized>,
}

/// One bullet under an experience entry.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bullet {
    /// The bullet text.
    pub text: Localized,
    /// See [`Item::id`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// See [`Item::profiles`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub profiles: Vec<String>,
    /// See [`Item::priority`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<Priority>,
    /// Where the fact comes from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}
item!(Bullet);

/// One position.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Experience {
    /// Employer.
    pub company: String,
    /// Job title, rendered before the company.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<Text>,
    /// City and country.
    pub location: String,
    /// Date range as printed.
    pub date: String,
    /// See [`Item::id`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// See [`Item::profiles`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub profiles: Vec<String>,
    /// See [`Item::priority`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<Priority>,
    /// Where the fact comes from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Bullets. An entry with none renders as a header line.
    #[serde(default)]
    pub bullets: Vec<Bullet>,
}
item!(Experience);

/// One degree or school.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Education {
    /// School or university.
    pub institution: String,
    /// Degree name.
    pub degree: Localized,
    /// Date range as printed.
    pub date: String,
    /// City and country.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    /// See [`Item::id`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// See [`Item::profiles`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub profiles: Vec<String>,
    /// See [`Item::priority`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<Priority>,
    /// Where the fact comes from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Bulleted details: thesis, grade, projects.
    #[serde(default)]
    pub details: Vec<LocalizedItem>,
}
item!(Education);

/// Items of a skill category.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SkillItems {
    /// One list for both languages; tool names do not translate.
    Shared(Vec<String>),
    /// A list per language, for spoken languages.
    PerLang {
        /// English list.
        en: Vec<String>,
        /// German list.
        de: Vec<String>,
    },
}

impl Default for SkillItems {
    fn default() -> Self {
        SkillItems::Shared(Vec::new())
    }
}

impl SkillItems {
    /// The list for `lang`.
    pub fn get(&self, lang: Lang) -> &[String] {
        match (self, lang) {
            (SkillItems::Shared(v), _) => v,
            (SkillItems::PerLang { en, .. }, Lang::En) => en,
            (SkillItems::PerLang { de, .. }, Lang::De) => de,
        }
    }
}

/// One skills row.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Skill {
    /// Row label.
    pub category: Localized,
    /// Row contents.
    pub items: SkillItems,
    /// See [`Item::id`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// See [`Item::profiles`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub profiles: Vec<String>,
    /// See [`Item::priority`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<Priority>,
    /// Where the fact comes from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}
item!(Skill);

/// A label / meta / detail row in a free section: an event, a talk, an award.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    /// Bold left column.
    pub label: Text,
    /// Right column, usually date and place.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<Text>,
    /// Line under the label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<Localized>,
    /// See [`Item::id`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// See [`Item::profiles`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub profiles: Vec<String>,
    /// See [`Item::priority`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<Priority>,
    /// Where the fact comes from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}
item!(Entry);

/// A bullet in a free section. `label` renders bold; neither field may carry markup.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SectionBullet {
    /// Bold lead-in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<Text>,
    /// The rest of the line.
    pub text: Localized,
    /// See [`Item::id`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// See [`Item::profiles`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub profiles: Vec<String>,
    /// See [`Item::priority`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<Priority>,
    /// Where the fact comes from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}
item!(SectionBullet);

/// A free section. It renders `intro`, `entries`, `bullets` and `prose`, in that order.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Section {
    /// Heading.
    pub title: Localized,
    /// See [`Item::id`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// See [`Item::profiles`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub profiles: Vec<String>,
    /// See [`Item::priority`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<Priority>,
    /// Where the fact comes from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Lead-in sentence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intro: Option<LocalizedItem>,
    /// Label / meta / detail rows.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<Entry>,
    /// Bullets.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bullets: Vec<SectionBullet>,
    /// Closing paragraph.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prose: Option<Localized>,
}
item!(Section);

/// The whole master file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Master {
    /// Languages a profile builds in. A profile not listed builds in every language.
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub profile_langs: IndexMap<String, Vec<Lang>>,
    /// Page limit per profile; 1 when absent, 0 for no limit.
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub page_limit: IndexMap<String, u32>,
    /// Full name.
    pub name: String,
    /// Contact line.
    pub contact: Contact,
    /// Title under the name, keyed by profile, with a `default`.
    pub title: IndexMap<String, Localized>,
    /// Summary paragraph, keyed by profile, with a `default`.
    pub summary: IndexMap<String, Localized>,
    /// LinkedIn headline and About.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linkedin: Option<LinkedinText>,
    /// Positions, newest first.
    #[serde(default)]
    pub experience: Vec<Experience>,
    /// Degrees and schools.
    #[serde(default)]
    pub education: Vec<Education>,
    /// Skill rows.
    #[serde(default)]
    pub skills: Vec<Skill>,
    /// Free sections.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sections: Vec<Section>,
}

impl Master {
    /// Parse and validate YAML text.
    ///
    /// # Errors
    /// The YAML does not parse, does not match the shape, or fails [`Master::validate`].
    pub fn from_yaml(text: &str) -> Result<Master, String> {
        let master: Master = serde_yaml_ng::from_str(text).map_err(|e| format!("master: {e}"))?;
        master.validate()?;
        Ok(master)
    }

    /// Serialize to YAML with a header that names the schema.
    ///
    /// # Errors
    /// Serialization fails, which a value built from this type does not do in practice.
    pub fn to_yaml(&self) -> Result<String, String> {
        let body = serde_yaml_ng::to_string(self).map_err(|e| e.to_string())?;
        Ok(format!(
            "# CV master. Shape and field meanings: capabilities/cv/master_cv.schema.yaml.\n{body}"
        ))
    }

    /// Check rules the types cannot express.
    ///
    /// # Errors
    /// `title` or `summary` has no `default`, a priority is outside 1..=3, or two items share an id.
    pub fn validate(&self) -> Result<(), String> {
        for (field, map) in [("title", &self.title), ("summary", &self.summary)] {
            if !map.contains_key("default") {
                return Err(format!("{field} needs a `default` entry"));
            }
        }
        let mut ids = BTreeSet::new();
        let mut problems = Vec::new();
        self.for_each_item(&mut |item| {
            if let Some(p) = item.priority() {
                if p.levels().iter().any(|l| !(1..=3).contains(l)) {
                    problems.push(format!("priority must be 1, 2 or 3 (item {:?})", item.id()));
                }
            }
            if let Some(id) = item.id() {
                if !ids.insert(id.to_string()) {
                    problems.push(format!("id '{id}' is used twice"));
                }
            }
        });
        match problems.into_iter().next() {
            Some(p) => Err(p),
            None => Ok(()),
        }
    }

    /// Visit every filterable item, depth first.
    pub fn for_each_item(&self, f: &mut dyn FnMut(&dyn Item)) {
        for e in &self.experience {
            f(e);
            e.bullets.iter().for_each(|b| f(b));
        }
        for e in &self.education {
            f(e);
            e.details.iter().for_each(|d| f(d));
        }
        self.skills.iter().for_each(|s| f(s));
        for s in &self.sections {
            f(s);
            if let Some(i) = &s.intro {
                f(i);
            }
            s.entries.iter().for_each(|e| f(e));
            s.bullets.iter().for_each(|b| f(b));
        }
    }

    /// Every profile tag in use, sorted: item tags, per-profile priorities, and title and
    /// summary variants. Always contains `default`.
    pub fn profiles(&self) -> Vec<String> {
        let mut tags = BTreeSet::from(["default".to_string()]);
        self.for_each_item(&mut |item| {
            tags.extend(item.profiles().iter().cloned());
            if let Some(Priority::PerProfile(map)) = item.priority() {
                tags.extend(map.keys().cloned());
            }
        });
        tags.extend(self.title.keys().cloned());
        tags.extend(self.summary.keys().cloned());
        tags.into_iter().collect()
    }

    /// Languages `profile` builds in.
    pub fn langs(&self, profile: &str) -> Vec<Lang> {
        self.profile_langs
            .get(profile)
            .cloned()
            .unwrap_or_else(|| Lang::ALL.to_vec())
    }

    /// Page limit of `profile`; 0 means no limit.
    pub fn page_limit(&self, profile: &str) -> u32 {
        self.page_limit.get(profile).copied().unwrap_or(1)
    }

    /// Headline for LinkedIn, falling back to the default title.
    pub fn headline(&self, lang: Lang) -> &str {
        self.linkedin
            .as_ref()
            .and_then(|l| l.headline.as_ref())
            .or_else(|| self.title.get("default"))
            .map_or("", |t| t.get(lang))
    }

    /// About text for LinkedIn, falling back to the default summary.
    pub fn about(&self, lang: Lang) -> &str {
        self.linkedin
            .as_ref()
            .and_then(|l| l.about.as_ref())
            .or_else(|| self.summary.get("default"))
            .map_or("", |t| t.get(lang))
    }
}

/// Where an application stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Not sent yet.
    Draft,
    /// Sent.
    Applied,
    /// In interviews.
    Interview,
    /// Offer received.
    Offer,
    /// Offer accepted.
    Accepted,
    /// Turned down by the company.
    Rejected,
    /// Withdrawn by the applicant.
    Withdrawn,
}

impl Status {
    /// Every status, in pipeline order.
    pub const ALL: [Status; 7] = [
        Status::Draft,
        Status::Applied,
        Status::Interview,
        Status::Offer,
        Status::Accepted,
        Status::Rejected,
        Status::Withdrawn,
    ];

    /// The lowercase name.
    pub fn name(self) -> &'static str {
        match self {
            Status::Draft => "draft",
            Status::Applied => "applied",
            Status::Interview => "interview",
            Status::Offer => "offer",
            Status::Accepted => "accepted",
            Status::Rejected => "rejected",
            Status::Withdrawn => "withdrawn",
        }
    }

    /// Parse a lowercase name.
    ///
    /// # Errors
    /// Any name not in [`Status::ALL`].
    pub fn parse(name: &str) -> Result<Status, String> {
        Status::ALL
            .into_iter()
            .find(|s| s.name() == name)
            .ok_or_else(|| {
                let all: Vec<_> = Status::ALL.iter().map(|s| s.name()).collect();
                format!("status must be one of: {}", all.join(" "))
            })
    }
}

/// One status change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogEntry {
    /// ISO date.
    pub date: String,
    /// Status set on that date.
    pub status: Status,
    /// Free text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// One application: what to build for it, and where it stands.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Application {
    /// Company.
    pub company: String,
    /// Role applied for.
    pub role: String,
    /// Posting URL.
    #[serde(default)]
    pub url: String,
    /// Profile filter to build with.
    #[serde(default = "default_profile")]
    pub profile: String,
    /// Build language.
    #[serde(default = "default_lang")]
    pub lang: Lang,
    /// Current status.
    pub status: Status,
    /// Status history, oldest first.
    #[serde(default)]
    pub log: Vec<LogEntry>,
    /// Replaces the profile's title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<Localized>,
    /// Replaces the profile's summary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<Localized>,
    /// Item ids always included.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pin: Vec<String>,
    /// Item ids never included.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hide: Vec<String>,
    /// Fit, gaps, contacts, interview preparation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

fn default_profile() -> String {
    "default".into()
}

fn default_lang() -> Lang {
    Lang::En
}

impl Application {
    /// Parse YAML text.
    ///
    /// # Errors
    /// The YAML does not parse or does not match the shape.
    pub fn from_yaml(text: &str) -> Result<Application, String> {
        serde_yaml_ng::from_str(text).map_err(|e| format!("application: {e}"))
    }

    /// Serialize to YAML.
    ///
    /// # Errors
    /// Serialization fails, which a value built from this type does not do in practice.
    pub fn to_yaml(&self) -> Result<String, String> {
        serde_yaml_ng::to_string(self).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCHEMA: &str = include_str!("../master_cv.schema.yaml");

    #[test]
    fn schema_skeleton_parses_and_round_trips() {
        let master = Master::from_yaml(SCHEMA).unwrap();
        let again = Master::from_yaml(&master.to_yaml().unwrap()).unwrap();
        assert_eq!(master, again);
    }

    #[test]
    fn a_misspelled_key_is_an_error() {
        let bad = SCHEMA.replacen("institution:", "institutoin:", 1);
        assert!(Master::from_yaml(&bad).is_err());
    }

    #[test]
    fn priority_resolves_per_profile_then_default() {
        let p: Priority = serde_yaml_ng::from_str("{default: 2, hackathon: 1}").unwrap();
        assert_eq!(p.level("hackathon"), 1);
        assert_eq!(p.level("swe"), 2);
        assert_eq!(Priority::Level(3).level("swe"), 3);
    }

    #[test]
    fn validation_refuses_duplicate_ids_and_bad_levels() {
        let mut m = Master::from_yaml(SCHEMA).unwrap();
        m.skills[0].id = Some("x".into());
        m.skills[1].id = Some("x".into());
        assert!(m.validate().unwrap_err().contains("twice"));
        m.skills[1].id = None;
        m.skills[1].priority = Some(Priority::Level(4));
        assert!(m.validate().is_err());
    }

    #[test]
    fn profiles_include_default_and_per_profile_priority_keys() {
        let mut m = Master::from_yaml(SCHEMA).unwrap();
        m.skills[0].priority = Some(serde_yaml_ng::from_str("{default: 2, quant: 1}").unwrap());
        let tags = m.profiles();
        assert!(tags.contains(&"default".to_string()));
        assert!(tags.contains(&"quant".to_string()));
    }

    #[test]
    fn missing_german_falls_back_to_english() {
        let l = Localized { en: "Hi".into(), de: None };
        assert_eq!(l.get(Lang::De), "Hi");
    }
}
