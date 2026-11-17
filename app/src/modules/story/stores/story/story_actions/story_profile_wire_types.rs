use rise_canon::rise_response;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StoryProfileScope {
    Profile,
    Archive,
}

impl StoryProfileScope {
    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Profile => "profile",
            Self::Archive => "archive",
        }
    }

    pub fn from_raw(raw: &str) -> Option<Self> {
        match raw {
            "profile" => Some(Self::Profile),
            "archive" => Some(Self::Archive),
            _ => None,
        }
    }
}

pub type GetProfileStoriesGatewayDTO = rise_response::story::GetProfileStories;
pub type GetCloseFriendsGatewayDTO = rise_response::story::GetCloseFriends;
