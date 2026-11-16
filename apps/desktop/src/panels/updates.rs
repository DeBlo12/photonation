// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! The Check for Updates window: what the latest release is, and the button
//! that installs it. What happens behind it is `updates`; see
//! docs/DESKTOP.md.

use crate::app::PhotonationApp;
use crate::panels::masks::hint;
use crate::theme;
use crate::updates::{Check, Finish, Method, Release, Stage, State, Version};
use eframe::egui::{self, RichText, Ui};
use std::time::Duration;

const WIDTH: f32 = 380.0;
const NOTES_HEIGHT: f32 = 180.0;
/// How often the progress bar looks at the download.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(200);

/// What a click in the window asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Choice {
    Close,
    CheckAgain,
    Install,
    /// Restart into the new version, or quit for the setup program.
    Finish,
    ShowInstaller,
}

pub fn show(app: &mut PhotonationApp, ctx: &egui::Context) {
    app.updates.poll();
    let mut choice = None;
    let modal = egui::Modal::new(egui::Id::unique("updates")).show(ctx, |ui| {
        ui.set_width(WIDTH);
        ui.label(
            RichText::new("Check for Updates")
                .font(theme::title(17.0))
                .color(theme::palette().text),
        );
        ui.add_space(10.0);
        choice = contents(app, ui);
    });
    if modal.should_close() {
        choice.get_or_insert(Choice::Close);
    }
    match choice {
        Some(Choice::Close) => app.view.show_updates = false,
        Some(Choice::CheckAgain) => app.updates.check(ctx),
        Some(Choice::Install) => app.updates.install(ctx),
        Some(Choice::Finish) => match app.updates.finish() {
            // `on_exit` saves the open photo's edits on the way out.
            Ok(()) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Err(message) => app.notify(message),
        },
        Some(Choice::ShowInstaller) => {
            if let Err(message) = app.updates.show_verified_installer() {
                app.notify(message);
            }
        }
        None => {}
    }
}

fn contents(app: &PhotonationApp, ui: &mut Ui) -> Option<Choice> {
    let running = Version::running();
    match app.updates.state() {
        State::Idle | State::Checking => {
            busy(ui, "Asking GitHub for the latest release…");
            buttons(ui, None, "Close")
        }
        State::Checked(Check::UpToDate) => {
            message(ui, &format!("Photonation {running} is the latest version."));
            privacy_note(ui);
            buttons(ui, None, "Close")
        }
        State::Checked(Check::NoRelease) => {
            message(
                ui,
                &format!("No release has been published yet. This copy is version {running}."),
            );
            privacy_note(ui);
            buttons(ui, None, "Close")
        }
        State::Checked(Check::Available(release)) => {
            message(
                ui,
                &format!(
                    "Photonation {} is available. This copy is version {running}.",
                    release.version
                ),
            );
            notes(ui, release);
            match why_not_installable(release, app.updates.method()) {
                None => {
                    hint(ui, how_it_installs(app.updates.method()));
                    buttons(ui, Some(("Install Update", Choice::Install)), "Not Now")
                }
                Some(reason) => {
                    hint(ui, reason);
                    buttons(ui, None, "Close")
                }
            }
        }
        State::CheckFailed(error) => {
            failure(ui, error);
            buttons(ui, Some(("Try Again", Choice::CheckAgain)), "Close")
        }
        State::Installing { release, stage, .. } => {
            let version = release.version;
            match stage {
                Stage::Downloading => {
                    message(ui, &format!("Downloading Photonation {version}…"));
                    let progress = app.updates.download_progress().unwrap_or(0.0);
                    ui.add(egui::ProgressBar::new(progress).desired_width(WIDTH));
                    ui.ctx().request_repaint_after(PROGRESS_INTERVAL);
                }
                Stage::Verifying => busy(ui, "Checking the download against its checksum…"),
                Stage::Installing => busy(ui, &format!("Installing Photonation {version}…")),
            }
            hint(ui, "This window can be closed; the update carries on.");
            buttons(ui, None, "Close")
        }
        State::Ready { release, finish } => {
            let version = release.version;
            let (text, button) = match finish {
                Finish::Relaunch { .. } => (
                    format!(
                        "Photonation {version} is installed. Restart to use it; your edits \
                         are saved first."
                    ),
                    "Restart Now",
                ),
                Finish::RunSetup { .. } => (
                    format!(
                        "The installer for Photonation {version} is downloaded and verified. \
                         Photonation quits and the installer opens; your edits are saved first."
                    ),
                    "Quit and Install",
                ),
            };
            message(ui, &text);
            buttons(ui, Some((button, Choice::Finish)), "Later")
        }
        State::InstallFailed {
            release,
            failure: stopped,
        } => {
            failure(ui, &stopped.message);
            if stopped.verified_installer.is_some() {
                hint(
                    ui,
                    "The download itself is complete and verified, and can be installed by hand.",
                );
                if ui
                    .link(RichText::new("Show the download").size(12.0))
                    .clicked()
                {
                    return Some(Choice::ShowInstaller);
                }
            }
            release_page_link(ui, release);
            buttons(ui, Some(("Try Again", Choice::Install)), "Close")
        }
    }
}

/// Why the window offers no Install button for `release`, if it doesn't.
fn why_not_installable(release: &Release, method: &Method) -> Option<&'static str> {
    if *method == Method::Unavailable {
        Some(
            "This copy wasn't put here by an installer, so it can't replace itself. Update \
             it from the source, or download the installer from the release's page.",
        )
    } else if release.installer.is_none() {
        Some(
            "This release has no installer for this computer that can be verified. See the \
             release's page.",
        )
    } else {
        None
    }
}

fn how_it_installs(method: &Method) -> &'static str {
    match method {
        Method::RunSetup => {
            "Downloads the installer from GitHub and checks it against its checksum. \
             Preferences, presets and models stay as they are."
        }
        Method::ReplaceBundle(_) | Method::Unavailable => {
            "Downloads the disk image from GitHub, checks it against its checksum and \
             replaces this app. Preferences, presets and models stay as they are."
        }
    }
}

fn message(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).color(theme::palette().text));
    ui.add_space(4.0);
}

fn busy(ui: &mut Ui, text: &str) {
    ui.horizontal(|ui| {
        ui.spinner();
        ui.label(RichText::new(text).color(theme::palette().text_secondary));
    });
    ui.add_space(4.0);
}

fn failure(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).color(theme::palette().accent_strong)).wrap());
    ui.add_space(4.0);
}

/// The check is the only time the app goes online, and says so.
fn privacy_note(ui: &mut Ui) {
    hint(
        ui,
        "Photonation only contacts GitHub when you choose Check for Updates.",
    );
}

/// What changed, as the release's page says it, and the way to that page.
fn notes(ui: &mut Ui, release: &Release) {
    if !release.notes.is_empty() {
        egui::Frame::NONE
            .fill(theme::palette().card)
            .corner_radius(theme::CONTROL_RADIUS)
            .inner_margin(egui::Margin::same(8))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(NOTES_HEIGHT)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new(&release.notes)
                                    .size(12.0)
                                    .color(theme::palette().text_secondary),
                            )
                            .wrap(),
                        );
                    });
            });
        ui.add_space(4.0);
    }
    release_page_link(ui, release);
    ui.add_space(4.0);
}

fn release_page_link(ui: &mut Ui, release: &Release) {
    ui.hyperlink_to(
        RichText::new("The release's page on GitHub").size(12.0),
        &release.page_url,
    );
}

/// The row of buttons at the bottom: the primary one, if any, rightmost.
fn buttons(ui: &mut Ui, primary: Option<(&str, Choice)>, dismiss: &str) -> Option<Choice> {
    let mut choice = None;
    ui.add_space(10.0);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if let Some((label, primary_choice)) = primary
            && ui
                .add(theme::primary_button(label).min_size(egui::vec2(84.0, 28.0)))
                .clicked()
        {
            choice = Some(primary_choice);
        }
        if ui
            .add(egui::Button::new(dismiss).min_size(egui::vec2(72.0, 28.0)))
            .clicked()
        {
            choice = Some(Choice::Close);
        }
    });
    choice
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::updates::tests::a_release;
    use crate::updates::{InstallFailure, Updater};

    fn mac_app() -> Method {
        Method::ReplaceBundle("/Applications/Photonation.app".into())
    }

    #[test]
    fn install_is_only_offered_where_it_can_work() {
        let release = a_release();
        assert_eq!(why_not_installable(&release, &mac_app()), None);
        assert_eq!(why_not_installable(&release, &Method::RunSetup), None);
        assert!(why_not_installable(&release, &Method::Unavailable).is_some());
        let without_installer = Release {
            installer: None,
            ..release
        };
        assert!(why_not_installable(&without_installer, &mac_app()).is_some());
    }

    /// Every state draws, with nothing to click and nothing asked of the
    /// network.
    #[test]
    fn the_window_draws_in_every_state() {
        let release = a_release();
        let relaunch = Finish::Relaunch {
            bundle: "/Applications/Photonation.app".into(),
        };
        let setup = Finish::RunSetup {
            setup: "setup.exe".into(),
        };
        let installing = |stage| State::Installing {
            release: release.clone(),
            stage,
            download: "missing".into(),
        };
        let states = [
            State::Idle,
            State::Checking,
            State::Checked(Check::UpToDate),
            State::Checked(Check::NoRelease),
            State::Checked(Check::Available(release.clone())),
            State::CheckFailed("Couldn't reach GitHub: offline".into()),
            installing(Stage::Downloading),
            installing(Stage::Verifying),
            installing(Stage::Installing),
            State::Ready {
                release: release.clone(),
                finish: relaunch,
            },
            State::Ready {
                release: release.clone(),
                finish: setup,
            },
            State::InstallFailed {
                release: release.clone(),
                failure: InstallFailure {
                    message: "Couldn't replace the app.".into(),
                    verified_installer: Some("photonation.dmg".into()),
                },
            },
        ];
        for state in states {
            for method in [mac_app(), Method::RunSetup, Method::Unavailable] {
                let ctx = egui::Context::default();
                let mut app = PhotonationApp::empty(&ctx);
                app.view.show_updates = true;
                app.updates = Updater::in_state(state.clone(), method);
                // The font texture a first frame uploads is not needed.
                ctx.run_ui(Default::default(), |ui| show(&mut app, ui.ctx()))
                    .textures_delta
                    .clear();
                assert!(app.view.show_updates, "{state:?} closed by itself");
                assert_eq!(app.updates.state(), &state);
            }
        }
    }
}
