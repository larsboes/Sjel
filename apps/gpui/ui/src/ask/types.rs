//! Internal turn models, exchange projections, and model targets for Ask sessions.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Role {
    User,
    Assistant,
    Error,
}

#[derive(Clone, Debug)]
pub(crate) struct Turn {
    pub(crate) role: Role,
    pub(crate) content: String,
    pub(crate) meta: Option<String>,
    pub(crate) tools: Vec<String>,
}

/// One question and its answer as it stands.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Exchange {
    pub prompt: String,
    pub answer: String,
    pub tools: Vec<String>,
    /// Which model answered, and from where.
    pub meta: Option<String>,
    pub error: Option<String>,
    pub loading: bool,
}

/// Where the next question goes, for the finder's footer.
#[derive(Clone, Debug, PartialEq)]
pub enum Model {
    /// The configured local model, with the reason the cloud roles could not be read, if any.
    Local { error: Option<String> },
    /// A cloud role. It receives the current prompt only, and only once `confirmed` (public c0).
    Cloud { label: String, confirmed: bool },
}

pub(crate) fn pairs(turns: &[Turn], loading: bool) -> Vec<Exchange> {
    let mut pairs: Vec<Exchange> = Vec::new();
    for turn in turns {
        if turn.role == Role::User {
            pairs.push(Exchange {
                prompt: turn.content.clone(),
                ..Exchange::default()
            });
            continue;
        }
        if pairs.is_empty() {
            pairs.push(Exchange::default());
        }
        let Some(pair) = pairs.last_mut() else {
            continue;
        };
        match turn.role {
            Role::Assistant => {
                pair.answer = turn.content.clone();
                pair.tools = turn.tools.clone();
                pair.meta = turn.meta.clone();
            }
            _ => pair.error = Some(turn.content.clone()),
        }
    }
    if let Some(last) = pairs.last_mut() {
        last.loading = loading;
    }
    pairs
}
