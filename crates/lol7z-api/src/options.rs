//! [`Options`]: the user's `-m` switches.

use crate::prop::Prop;

/// One `-m` switch, such as `-mx=9`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Opt {
    pub name: String,
    /// `None` for a switch given without a value (`-mx`).
    pub value: Option<Prop<'static>>,
}

/// The `-m` switches the user gave, passed to [`Format::open`](crate::Format::open) and [`Update::update`](crate::Update::update).
/// Unknown names are accepted and kept.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Options(pub Vec<Opt>);

impl Options {
    /// The last switch with this name, ignoring ASCII case as 7-Zip does.
    pub fn get(&self, name: &str) -> Option<&Opt> {
        self.0
            .iter()
            .rev()
            .find(|o| o.name.eq_ignore_ascii_case(name))
    }
}
