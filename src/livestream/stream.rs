/// Type of stream
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Stream {
    Main,

    // Alternative media
    Video { name: String, lang: Option<String> },
    Audio { name: String, lang: Option<String> },
    Subtitle { name: String, lang: Option<String> },
}
