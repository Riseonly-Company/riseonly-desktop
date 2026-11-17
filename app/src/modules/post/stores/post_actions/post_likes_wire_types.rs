use rise_canon::rise_proto::post::PostLikeUser as CanonPostLikeUser;
use rise_canon::rise_response::post::GetPostLikes;

use crate::modules::post::stores::post_actions::post_likes_types::PostLikeUser;

pub type PostLikeUserDTO = CanonPostLikeUser;
pub type GetPostLikesGatewayDTO = GetPostLikes;

/// converts the like user dto to the domain model
pub trait PostLikeUserDTOExt {
    fn as_domain(&self) -> PostLikeUser;
}

impl PostLikeUserDTOExt for PostLikeUserDTO {
    fn as_domain(&self) -> PostLikeUser {
        PostLikeUser {
            id: self.id.clone(),
            name: self.name.clone(),
            tag: self.tag.clone(),
            logo: self.logo.clone(),
            is_premium: self.is_premium,
            is_official: self.is_official,
            is_friend: self.is_friend,
            is_subbed: self.is_subbed,
            is_subscriber: self.is_subscriber,
            liked_at_ms: self.liked_at_ms,
            premium_badge_emoji: self.premium_badge_emoji.clone(),
            name_color: self.name_color.clone(),
        }
    }
}
