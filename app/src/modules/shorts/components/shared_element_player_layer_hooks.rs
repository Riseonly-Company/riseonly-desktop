use std::cell::Cell;
use std::rc::Rc;

use gpui::{App, Bounds, ElementId, Entity, IntoElement, Pixels, Styled, WeakEntity, canvas, point, px, size};
use rise_media::video::av_player::{AVLayerVideoGravity, AVPlayer};

pub struct SharedElementPlayerLayerViewInner {
    pub player: WeakEntity<AVPlayer>,
    video_gravity: AVLayerVideoGravity,
    pub frame: Cell<Option<Bounds<Pixels>>>,
    is_hidden: Cell<bool>,
}

/// desktop stand in for a view whose layer is a player layer
#[derive(Clone)]
pub struct SharedElementPlayerLayerView {
    pub inner: Rc<SharedElementPlayerLayerViewInner>,
}

impl SharedElementPlayerLayerView {
    /// true only for the same published layer, an equal one doesnt count
    pub fn same_layer(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.inner, &other.inner)
    }

    pub fn new(player: &Entity<AVPlayer>, video_gravity: AVLayerVideoGravity) -> Self {
        Self {
            inner: Rc::new(SharedElementPlayerLayerViewInner {
                player: player.downgrade(),
                video_gravity,
                frame: Cell::new(None),
                is_hidden: Cell::new(false),
            }),
        }
    }

    pub fn set_hidden(&self, hidden: bool) {
        self.inner.is_hidden.set(hidden);
    }

    pub fn is_hidden(&self) -> bool {
        self.inner.is_hidden.get()
    }

    /// layer frame in window space, none while its not in a window
    pub fn frame(&self) -> Option<Bounds<Pixels>> {
        self.inner.frame.get()
    }

    pub fn player(&self) -> Option<Entity<AVPlayer>> {
        self.inner.player.upgrade()
    }

    /// zero hit test element that fills the player layer frame, draw it inside that frame
    pub fn probe(&self, id: impl Into<ElementId>) -> impl IntoElement {
        let id: ElementId = id.into();
        let view = self.clone();
        canvas(
            move |bounds, window, _| {
                window.with_global_id(id, |global_id, window| {
                    window.with_element_state::<SharedElementPlayerLayerAttachment, _>(global_id, |attachment, window| {
                        let attachment = match attachment {
                            Some(attachment) if Rc::ptr_eq(&attachment.view, &view.inner) => attachment,
                            _ => SharedElementPlayerLayerAttachment {
                                view: view.inner.clone(),
                            },
                        };
                        view.inner.frame.set(Some(bounds));
                        let moved = view.inner.clone();
                        window.on_replayed_move(move |offset, _, _, _| moved.frame.set(Some(bounds + offset)));
                        ((), attachment)
                    })
                });
            },
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
    }

    /// where the picture sits in the layer frame, none while the presentation size is unknown
    pub fn video_rect(&self, frame: Bounds<Pixels>, cx: &App) -> Option<Bounds<Pixels>> {
        let presentation = self.player()?.read(cx).presentation_size();
        if presentation.width <= 0.0 || presentation.height <= 0.0 {
            return None;
        }
        let width = f32::from(frame.size.width);
        let height = f32::from(frame.size.height);
        let scale = match self.inner.video_gravity {
            AVLayerVideoGravity::ResizeAspect => (width / presentation.width).min(height / presentation.height),
            AVLayerVideoGravity::ResizeAspectFill => (width / presentation.width).max(height / presentation.height),
            AVLayerVideoGravity::Resize => return Some(frame),
        };
        let video_width = presentation.width * scale;
        let video_height = presentation.height * scale;
        Some(Bounds::new(
            point(
                frame.origin.x + px((width - video_width) / 2.0),
                frame.origin.y + px((height - video_height) / 2.0),
            ),
            size(px(video_width), px(video_height)),
        ))
    }
}

/// element state of a drawn probe, gpui drops it after the first frame that skips the probe
/// meaning the layer left the window
struct SharedElementPlayerLayerAttachment {
    view: Rc<SharedElementPlayerLayerViewInner>,
}

impl Drop for SharedElementPlayerLayerAttachment {
    fn drop(&mut self) {
        self.view.frame.set(None);
    }
}
