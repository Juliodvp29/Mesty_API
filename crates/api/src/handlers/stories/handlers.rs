use crate::handlers::stories::StoriesState;
use crate::handlers::stories::dto::{
    CreateStoryRequest, CreateStoryResponse, GroupedStoriesResponse, ReactToStoryRequest,
    StoryViewResponse, StoryWithUserResponse, UserStoryItem,
};
use crate::middleware::auth::AuthenticatedUser;
use axum::{
    Json,
    extract::{Extension, Path, State},
};
use shared::error::DomainError;
use uuid::Uuid;

use crate::error::ApiError;
use crate::services::push::{PRESENCE_KEY_PREFIX, PushNotificationJob, enqueue_push_notification};
use domain::chat::notifications::{NewNotification, NotificationType};
use domain::chat::repository::ChatRepository;
use redis::AsyncCommands;

const VALID_PRIVACY_VALUES: [&str; 5] = [
    "everyone",
    "contacts",
    "contacts_except",
    "only_me",
    "selected",
];

fn is_valid_privacy(v: &str) -> bool {
    VALID_PRIVACY_VALUES.contains(&v)
}

pub async fn create_story(
    State(state): State<StoriesState>,
    Extension(auth): Extension<AuthenticatedUser>,
    Json(req): Json<CreateStoryRequest>,
) -> Result<Json<CreateStoryResponse>, ApiError> {
    if !is_valid_privacy(&req.privacy) {
        return Err(ApiError(DomainError::Validation(
            "Invalid privacy value. Must be one of: everyone, contacts, contacts_except, only_me, selected".to_string(),
        )));
    }

    let story = state
        .story_repo
        .create(
            auth.user_id,
            req.content_url,
            req.content_type,
            req.caption,
            req.privacy.clone(),
        )
        .await
        .map_err(|e| ApiError(DomainError::from_boxed_err(e)))?;

    if let Some(exceptions) = req.exceptions {
        for user_id in exceptions {
            let is_excluded = req.privacy == "contacts_except";
            state
                .story_repo
                .add_privacy_exception(story.id, user_id, is_excluded)
                .await
                .map_err(|e| ApiError(DomainError::from_boxed_err(e)))?;
        }
    }

    Ok(Json(CreateStoryResponse {
        id: story.id,
        expires_at: story.expires_at.to_rfc3339(),
    }))
}

/// GET /stories — returns stories grouped by user, unseen first.
pub async fn list_stories(
    State(state): State<StoriesState>,
    Extension(auth): Extension<AuthenticatedUser>,
) -> Result<Json<Vec<GroupedStoriesResponse>>, ApiError> {
    let stories = state
        .story_repo
        .list_for_user(auth.user_id)
        .await
        .map_err(|e| ApiError(DomainError::from_boxed_err(e)))?;

    // Group stories by user_id preserving the query order (unseen first).
    let mut grouped: Vec<GroupedStoriesResponse> = Vec::new();

    for s in stories {
        let pos = grouped.iter().position(|g| g.user_id == s.user_id);

        let group = if let Some(idx) = pos {
            &mut grouped[idx]
        } else {
            grouped.push(GroupedStoriesResponse {
                user_id: s.user_id,
                username: s.username.clone(),
                display_name: s.display_name.clone(),
                avatar_url: s.avatar_url.clone(),
                stories: Vec::new(),
            });
            grouped.last_mut().unwrap()
        };

        group.stories.push(UserStoryItem {
            id: s.id,
            content_url: s.content_url,
            content_type: s.content_type,
            caption: s.caption,
            privacy: s.privacy,
            created_at: s.created_at.to_rfc3339(),
            expires_at: s.expires_at.to_rfc3339(),
            has_viewed: s.has_viewed,
        });
    }

    Ok(Json(grouped))
}

pub async fn list_my_stories(
    State(state): State<StoriesState>,
    Extension(auth): Extension<AuthenticatedUser>,
) -> Result<Json<Vec<StoryWithUserResponse>>, ApiError> {
    let stories = state
        .story_repo
        .list_my_stories(auth.user_id)
        .await
        .map_err(|e| ApiError(DomainError::from_boxed_err(e)))?;

    let response: Vec<StoryWithUserResponse> = stories
        .into_iter()
        .map(|s| StoryWithUserResponse {
            id: s.id,
            user_id: s.user_id,
            content_url: s.content_url,
            content_type: s.content_type,
            caption: s.caption,
            privacy: s.privacy.clone(),
            created_at: s.created_at.to_rfc3339(),
            expires_at: s.expires_at.to_rfc3339(),
            username: s.username,
            display_name: s.display_name,
            avatar_url: s.avatar_url,
            has_viewed: s.has_viewed,
        })
        .collect();

    Ok(Json(response))
}

pub async fn delete_story(
    State(state): State<StoriesState>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(story_id): Path<Uuid>,
) -> Result<Json<()>, ApiError> {
    let story = state
        .story_repo
        .find_by_id(story_id)
        .await
        .map_err(|e| ApiError(DomainError::from_boxed_err(e)))?
        .ok_or_else(|| ApiError(DomainError::NotFound("Story not found".to_string())))?;

    if story.user_id != auth.user_id {
        return Err(ApiError(DomainError::Unauthorized(
            "Cannot delete another user's story".to_string(),
        )));
    }

    state
        .story_repo
        .delete(story_id, auth.user_id)
        .await
        .map_err(|e| ApiError(DomainError::from_boxed_err(e)))?;

    Ok(Json(()))
}

/// POST /stories/:id/view — registers a view. No body needed.
/// Validates that the viewer has permission based on the story's privacy settings.
pub async fn view_story(
    State(state): State<StoriesState>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(story_id): Path<Uuid>,
) -> Result<Json<()>, ApiError> {
    let story = state
        .story_repo
        .find_by_id(story_id)
        .await
        .map_err(|e| ApiError(DomainError::from_boxed_err(e)))?
        .ok_or_else(|| ApiError(DomainError::NotFound("Story not found".to_string())))?;

    if story.user_id == auth.user_id {
        // If it's the author, return success but don't register a view or notify
        return Ok(Json(()));
    }

    // Verify privacy permissions
    let can_view = state
        .story_repo
        .can_user_view_story(story_id, auth.user_id)
        .await
        .map_err(|e| ApiError(DomainError::from_boxed_err(e)))?;

    if !can_view {
        return Err(ApiError(DomainError::Unauthorized(
            "You do not have permission to view this story".to_string(),
        )));
    }

    state
        .story_repo
        .mark_viewed(story_id, auth.user_id)
        .await
        .map_err(|e| ApiError(DomainError::from_boxed_err(e)))?;

    let notification = NewNotification {
        user_id: story.user_id,
        notification_type: NotificationType::StoryView,
        data: serde_json::json!({
            "story_id": story.id,
            "viewer_id": auth.user_id,
        }),
    };

    if let Err(e) = state.chat_repo.create_notification(notification).await {
        tracing::error!("Failed to create in-app notification: {}", e);
    }

    let mut redis = state.redis.clone();
    let presence_key = format!("{}{}", PRESENCE_KEY_PREFIX, story.user_id);
    let is_online: bool = redis.exists(&presence_key).await.unwrap_or(false);

    if !is_online {
        let job = PushNotificationJob {
            user_id: story.user_id,
            notification_type: "story_view".to_string(),
            payload: serde_json::json!({
                "story_id": story.id,
                "viewer_id": auth.user_id,
            }),
        };

        if let Err(e) = enqueue_push_notification(&mut redis, job).await {
            tracing::error!("Failed to enqueue push notification: {}", e);
        }
    }

    Ok(Json(()))
}

/// POST /stories/:id/react — adds a reaction. Requires prior view.
/// Validates privacy permissions before allowing the reaction.
pub async fn react_to_story(
    State(state): State<StoriesState>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(story_id): Path<Uuid>,
    Json(req): Json<ReactToStoryRequest>,
) -> Result<Json<()>, ApiError> {
    let story = state
        .story_repo
        .find_by_id(story_id)
        .await
        .map_err(|e| ApiError(DomainError::from_boxed_err(e)))?
        .ok_or_else(|| ApiError(DomainError::NotFound("Story not found".to_string())))?;

    if story.user_id == auth.user_id {
        // Don't allow self-reactions, but return success silently
        return Ok(Json(()));
    }

    // Verify privacy permissions
    let can_view = state
        .story_repo
        .can_user_view_story(story_id, auth.user_id)
        .await
        .map_err(|e| ApiError(DomainError::from_boxed_err(e)))?;

    if !can_view {
        return Err(ApiError(DomainError::Unauthorized(
            "You do not have permission to interact with this story".to_string(),
        )));
    }

    let has_viewed = state
        .story_repo
        .has_viewed(story_id, auth.user_id)
        .await
        .map_err(|e| ApiError(DomainError::from_boxed_err(e)))?;

    if !has_viewed {
        return Err(ApiError(DomainError::Validation(
            "Cannot react to a story you haven't viewed".to_string(),
        )));
    }

    state
        .story_repo
        .add_reaction(story_id, auth.user_id, req.reaction.clone())
        .await
        .map_err(|e| ApiError(DomainError::from_boxed_err(e)))?;

    let notification = NewNotification {
        user_id: story.user_id,
        notification_type: NotificationType::StoryReaction,
        data: serde_json::json!({
            "story_id": story.id,
            "reactor_id": auth.user_id,
            "reaction": req.reaction,
        }),
    };

    if let Err(e) = state.chat_repo.create_notification(notification).await {
        tracing::error!("Failed to create in-app notification: {}", e);
    }

    let mut redis = state.redis.clone();
    let presence_key = format!("{}{}", PRESENCE_KEY_PREFIX, story.user_id);
    let is_online: bool = redis.exists(&presence_key).await.unwrap_or(false);

    if !is_online {
        let job = PushNotificationJob {
            user_id: story.user_id,
            notification_type: "story_reaction".to_string(),
            payload: serde_json::json!({
                "story_id": story.id,
                "reactor_id": auth.user_id,
                "reaction": req.reaction,
            }),
        };

        if let Err(e) = enqueue_push_notification(&mut redis, job).await {
            tracing::error!("Failed to enqueue push notification: {}", e);
        }
    }

    Ok(Json(()))
}

pub async fn get_story_views(
    State(state): State<StoriesState>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(story_id): Path<Uuid>,
) -> Result<Json<Vec<StoryViewResponse>>, ApiError> {
    let story = state
        .story_repo
        .find_by_id(story_id)
        .await
        .map_err(|e| ApiError(DomainError::from_boxed_err(e)))?
        .ok_or_else(|| ApiError(DomainError::NotFound("Story not found".to_string())))?;

    if story.user_id != auth.user_id {
        return Err(ApiError(DomainError::Unauthorized(
            "Only the story author can view viewers".to_string(),
        )));
    }

    let views = state
        .story_repo
        .get_views_with_user(story_id)
        .await
        .map_err(|e| ApiError(DomainError::from_boxed_err(e)))?;

    let response: Vec<StoryViewResponse> = views
        .into_iter()
        .map(|v| StoryViewResponse {
            viewer_id: v.viewer_id,
            display_name: v.display_name,
            avatar_url: v.avatar_url,
            reaction: v.reaction,
            viewed_at: v.viewed_at.to_rfc3339(),
        })
        .collect();

    Ok(Json(response))
}
