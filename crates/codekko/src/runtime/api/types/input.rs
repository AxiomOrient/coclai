pub type ThreadId = String;
pub type TurnId = String;
pub type ItemId = String;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputItem {
    Text {
        text: String,
    },
    TextWithElements {
        text: String,
        text_elements: Vec<TextElement>,
    },
    ImageUrl {
        url: String,
    },
    LocalImage {
        path: String,
    },
    Skill {
        name: String,
        path: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ByteRange {
    pub start: u64,
    pub end: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextElement {
    pub byte_range: ByteRange,
    pub placeholder: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PromptAttachment {
    AtPath {
        path: String,
        placeholder: Option<String>,
    },
    ImageUrl {
        url: String,
    },
    LocalImage {
        path: String,
    },
    Skill {
        name: String,
        path: String,
    },
}
