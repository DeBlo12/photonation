// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Liquid Glass behind the window, the inspector and the filmstrip, on macOS 26
//! and later.
//!
//! egui draws everything into one Metal layer, so the glass cannot sit between
//! its widgets. Instead each pane gets an `NSGlassEffectView` behind that layer,
//! kept on the rectangle egui reports, and egui leaves those rectangles see-through.
//! The glass then bends whatever is behind the window: the desktop and other apps.
//! winit expects its own view to stay the window's content view, so the glass
//! views are siblings below it. See docs/DESKTOP.md.

use eframe::egui::{Rect, Theme};
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2::runtime::AnyClass;
use objc2_app_kit::{
    NSAppearance, NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSApplication,
    NSAutoresizingMaskOptions, NSGlassEffectView, NSScreen, NSView, NSWindow, NSWindowOrderingMode,
    NSWindowStyleMask,
};
use objc2_foundation::{NSPoint, NSRect, NSSize};

/// Whether this macOS has the glass view at all. Decides, before the window
/// exists, whether it is created transparent.
pub fn supported() -> bool {
    AnyClass::get(c"NSGlassEffectView").is_some()
}

/// Whether macOS has the window in full screen, from the window itself, which
/// also knows when the user took it there or back through the system. `None`
/// without a window on this thread.
pub fn window_is_full_screen() -> Option<bool> {
    let main_thread = MainThreadMarker::new()?;
    let window = NSApplication::sharedApplication(main_thread)
        .windows()
        .firstObject()?;
    Some(window.styleMask().contains(NSWindowStyleMask::FullScreen))
}

/// Makes the glass and native dialogs match a mode other than the system's, as
/// egui then does. `None` hands them back to the system.
pub fn force_appearance(theme: Option<Theme>) {
    let Some(main_thread) = MainThreadMarker::new() else {
        return;
    };
    let appearance = theme.and_then(|theme| {
        // SAFETY: the names are AppKit's constants, valid while AppKit is loaded.
        let name = unsafe {
            match theme {
                Theme::Dark => NSAppearanceNameDarkAqua,
                Theme::Light => NSAppearanceNameAqua,
            }
        };
        NSAppearance::appearanceNamed(name)
    });
    NSApplication::sharedApplication(main_thread).setAppearance(appearance.as_deref());
}

/// The corner radius of a glass pane of this size.
fn pane_radius(pane: Rect) -> f32 {
    f32::from(crate::theme::INSPECTOR_RADIUS).min(pane.height() / 2.0)
}

/// One glass view and where it was last put.
struct GlassPane {
    view: Retained<NSGlassEffectView>,
    placed: Option<Rect>,
}

impl GlassPane {
    /// Shows the pane on `rect` (egui points, from the top left), or hides it.
    fn place(&mut self, rect: Option<Rect>, corner_radius: f32, parent_height: f64) -> bool {
        if rect == self.placed {
            return false;
        }
        self.view.setCornerRadius(corner_radius.into());
        self.placed = rect;
        self.view.setHidden(rect.is_none());
        if let Some(rect) = rect {
            self.view.setFrame(NSRect::new(
                NSPoint::new(rect.left().into(), parent_height - f64::from(rect.bottom())),
                NSSize::new(rect.width().into(), rect.height().into()),
            ));
        }
        true
    }
}

/// The glass views of the main window.
pub struct GlassPanes {
    window: Retained<NSWindow>,
    parent: Retained<NSView>,
    /// Behind the whole window, for as long as the window exists.
    _background: Retained<NSGlassEffectView>,
    inspector: GlassPane,
    filmstrip: GlassPane,
}

impl GlassPanes {
    /// Puts hidden glass views behind the window's content. `None` where macOS
    /// has no glass or the window is not as expected; the app then stays opaque.
    pub fn install() -> Option<GlassPanes> {
        if !supported() {
            return None;
        }
        let main_thread = MainThreadMarker::new()?;
        let window = NSApplication::sharedApplication(main_thread)
            .windows()
            .firstObject()?;
        let content = window.contentView()?;
        // SAFETY: both views belong to this window, on the main thread.
        let parent = unsafe { content.superview() }?;
        let glass_view = || {
            let view = NSGlassEffectView::new(main_thread);
            parent.addSubview_positioned_relativeTo(
                &view,
                NSWindowOrderingMode::Below,
                Some(&content),
            );
            view
        };
        // Each view goes directly below the content, so above the ones before.
        let background = glass_view();
        background.setFrame(parent.bounds());
        background.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        let pane = || {
            let view = glass_view();
            view.setHidden(true);
            GlassPane { view, placed: None }
        };
        let inspector = pane();
        let filmstrip = pane();
        Some(GlassPanes {
            window,
            parent,
            _background: background,
            inspector,
            filmstrip,
        })
    }

    /// Moves the panes to where egui drew them this frame. `None` hides one.
    pub fn place(&mut self, inspector: Option<Rect>, filmstrip: Option<Rect>) {
        let radius_of = |rect: Option<Rect>| rect.map_or(0.0, pane_radius);
        let height = self.parent.bounds().size.height;
        let inspector_moved = self
            .inspector
            .place(inspector, radius_of(inspector), height);
        let filmstrip_moved = self
            .filmstrip
            .place(filmstrip, radius_of(filmstrip), height);
        if inspector_moved || filmstrip_moved {
            // macOS derives the window's shadow from what it shows.
            self.window.invalidateShadow();
        }
    }

    /// Brings the window in front without taking the keyboard, and returns
    /// where it is on the main screen as `[x, y, width, height]` from the top
    /// left, to capture it with what shows through.
    pub fn front_for_capture(&self) -> Option<[f64; 4]> {
        self.window.orderFrontRegardless();
        let main_thread = MainThreadMarker::new()?;
        let screen_height = NSScreen::mainScreen(main_thread)?.frame().size.height;
        let frame = self.window.frame();
        Some([
            frame.origin.x,
            screen_height - frame.origin.y - frame.size.height,
            frame.size.width,
            frame.size.height,
        ])
    }
}
