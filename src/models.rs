#[derive(Clone)]
pub enum TextInPost {
    Post(String),
    Picture(String),
    Video(String),
    //Stuff like an blog post url with alt, title, desc
    External(String),
}

impl TextInPost {
    pub fn to_string(self) -> String {
        //Hack gotta be a easier way
        match self {
            TextInPost::Post(value) => value,
            TextInPost::Picture(value) => value,
            TextInPost::Video(value) => value,
            TextInPost::External(value) => value,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PostScoring {
    pub pinned: bool,
    pub deleted: bool,
    pub priority: i64,
}

pub struct DbPost {
    pub uri: String,
    pub text: String,
    pub pinned: bool,
    pub deleted: bool,
    pub priority: i64,
    // pub timestamp: DateTime<Utc>,
}
