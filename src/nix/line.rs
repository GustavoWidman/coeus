use std::{borrow::Cow, fmt, marker::PhantomData};

use serde::{
    Deserialize, Deserializer,
    de::{self, IgnoredAny, SeqAccess, Visitor},
};
use serde_json::value::RawValue;

/// one line of `nix ... --log-format internal-json`, borrowed from the raw text
#[derive(Clone, Debug, PartialEq)]
pub enum Line<'a> {
    Nix(Event<'a>),
    /// anything that is not an `@nix` event, kept as written
    Output(&'a str),
}

impl<'a> Line<'a> {
    const PREFIX: &'static str = "@nix ";

    /// never fails: a line that does not hold a well-formed event is output
    pub fn parse(raw: &'a str) -> Self {
        let raw = raw.strip_suffix('\n').unwrap_or(raw);
        raw.strip_prefix(Self::PREFIX)
            .and_then(|json| serde_json::from_str::<Wire<'a>>(json).ok())
            .and_then(Wire::into_event)
            .map_or(Self::Output(raw), Self::Nix)
    }

    pub const fn event(&self) -> Option<&Event<'a>> {
        match self {
            Self::Nix(event) => Some(event),
            Self::Output(_) => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event<'a> {
    Start(Start<'a>),
    Stop(ActivityId),
    Result(Report<'a>),
    Msg(Msg<'a>),
}

/// nix numbers its activities per process; `0` means no activity
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ActivityId(pub u64);

#[derive(Clone, Debug, PartialEq)]
pub struct Start<'a> {
    pub id: ActivityId,
    pub parent: Option<ActivityId>,
    pub activity: Activity,
    pub level: Level,
    pub text: Cow<'a, str>,
    pub fields: Fields<'a>,
}

impl Start<'_> {
    /// the derivation a [Activity::Build] builds
    pub fn drv_path(&self) -> Option<Cow<'_, str>> {
        match self.activity {
            Activity::Build => self.fields.first_str(),
            _ => None,
        }
    }
}

/// a `result` event: something an activity reports while it runs
#[derive(Clone, Debug, PartialEq)]
pub struct Report<'a> {
    pub id: ActivityId,
    pub kind: ResultKind,
    pub fields: Fields<'a>,
}

impl Report<'_> {
    /// `[done, expected, running, failed]`, in the unit of the activity
    pub fn progress(&self) -> Option<Counts> {
        match self.kind {
            ResultKind::Progress => self.fields.parse::<(u64, u64, u64, u64)>().map(
                |(done, expected, running, failed)| Counts {
                    done,
                    expected,
                    running,
                    failed,
                },
            ),
            _ => None,
        }
    }

    /// how much work of one kind the activity expects in total
    pub fn expected(&self) -> Option<(Activity, u64)> {
        match self.kind {
            ResultKind::SetExpected => self
                .fields
                .parse::<(u64, u64)>()
                .map(|(activity, expected)| (Activity::from_code(activity), expected)),
            _ => None,
        }
    }

    /// one line a builder printed
    pub fn log_line(&self) -> Option<Cow<'_, str>> {
        match self.kind {
            ResultKind::BuildLogLine => self.fields.first_str(),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub done: u64,
    pub expected: u64,
    pub running: u64,
    pub failed: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Msg<'a> {
    pub level: Level,
    pub text: Cow<'a, str>,
}

/// the untouched `fields` array of an event, decoded only when asked
#[derive(Clone, Copy, Debug, Default)]
pub struct Fields<'a>(Option<&'a RawValue>);

impl<'a> Fields<'a> {
    pub fn parse<T: Deserialize<'a>>(&self) -> Option<T> {
        serde_json::from_str(self.0?.get()).ok()
    }

    pub fn first_str(&self) -> Option<Cow<'a, str>> {
        self.parse::<Head<Text<'a>>>().map(|Head(Text(text))| text)
    }

    pub fn as_str(&self) -> &'a str {
        self.0.map_or("", RawValue::get)
    }
}

impl PartialEq for Fields<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

/// how loud nix considers a message; lower is more important
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    Error,
    Warn,
    Notice,
    Info,
    Talkative,
    Chatty,
    Debug,
    Vomit,
}

impl Level {
    const fn from_code(code: u8) -> Self {
        match code {
            0 => Self::Error,
            1 => Self::Warn,
            2 => Self::Notice,
            3 => Self::Info,
            4 => Self::Talkative,
            5 => Self::Chatty,
            6 => Self::Debug,
            _ => Self::Vomit,
        }
    }
}

macro_rules! codes {
    ($(#[$meta:meta])* $name:ident { $($variant:ident = $code:literal),* $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum $name {
            $($variant,)*
            /// a code this version of coeus does not know, such as Determinate
            /// Nix's own activities
            Other(u64),
        }

        impl $name {
            pub const fn from_code(code: u64) -> Self {
                match code {
                    $($code => Self::$variant,)*
                    other => Self::Other(other),
                }
            }
        }
    };
}

codes! {
    /// what an activity does
    Activity {
        Unknown = 0,
        CopyPath = 100,
        FileTransfer = 101,
        Realise = 102,
        CopyPaths = 103,
        Builds = 104,
        Build = 105,
        OptimiseStore = 106,
        VerifyPaths = 107,
        Substitute = 108,
        QueryPathInfo = 109,
        PostBuildHook = 110,
        BuildWaiting = 111,
        FetchTree = 112,
    }
}

codes! {
    /// what a `result` event reports
    ResultKind {
        FileLinked = 100,
        BuildLogLine = 101,
        UntrustedPath = 102,
        CorruptedPath = 103,
        SetPhase = 104,
        Progress = 105,
        SetExpected = 106,
        PostBuildLogLine = 107,
        FetchStatus = 108,
    }
}

/// a string that borrows from the line unless it holds escapes
#[derive(Deserialize)]
struct Text<'a>(#[serde(borrow)] Cow<'a, str>);

/// the first element of an array, whatever follows it
struct Head<T>(T);

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Head<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visit<T>(PhantomData<T>);

        impl<'de, T: Deserialize<'de>> Visitor<'de> for Visit<T> {
            type Value = Head<T>;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a non-empty array")
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Head<T>, A::Error> {
                let head = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(0, &self))?;
                while seq.next_element::<IgnoredAny>()?.is_some() {}
                Ok(Head(head))
            }
        }

        deserializer.deserialize_seq(Visit(PhantomData))
    }
}

/// the union of every event's keys; serde skips the ones nix adds later
#[derive(Deserialize)]
struct Wire<'a> {
    #[serde(borrow)]
    action: Cow<'a, str>,
    #[serde(default)]
    id: u64,
    #[serde(default)]
    parent: u64,
    #[serde(default, rename = "type")]
    kind: u64,
    #[serde(default)]
    level: u8,
    #[serde(borrow, default)]
    text: Cow<'a, str>,
    #[serde(borrow, default)]
    msg: Cow<'a, str>,
    #[serde(borrow, default)]
    fields: Option<&'a RawValue>,
}

impl<'a> Wire<'a> {
    fn into_event(self) -> Option<Event<'a>> {
        let id = ActivityId(self.id);
        let fields = Fields(self.fields);

        Some(match &*self.action {
            "start" => Event::Start(Start {
                id,
                parent: (self.parent != 0).then_some(ActivityId(self.parent)),
                activity: Activity::from_code(self.kind),
                level: Level::from_code(self.level),
                text: self.text,
                fields,
            }),
            "stop" => Event::Stop(id),
            "result" => Event::Result(Report {
                id,
                kind: ResultKind::from_code(self.kind),
                fields,
            }),
            "msg" => Event::Msg(Msg {
                level: Level::from_code(self.level),
                text: self.msg,
            }),
            _ => return None,
        })
    }
}
