//! The dialogs that take a file or a folder (the Go app's `vms.go` import/export/move and
//! `install.go` offline installers): Import, Export, Move, Offline installers and the progress
//! dialog of a long copy. Each has a path field, so a path can always be typed; Browse… opens
//! the XDG desktop portal's file chooser (`ashpd`, rustXterm's pattern) and, when no portal
//! answers, says so and leaves the field to type in.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use ashpd::desktop::file_chooser::{FileFilter, SelectedFiles};
use egui::{RichText, Ui, vec2};
use rg_core::registry::slug;

use crate::app::{Action, App};
use crate::backend::{Installer, Task, VmEntry};
use crate::dialogs::{Modal, Outcome};
use crate::icons;
use crate::widgets::{self, Footer, bold, caption, form_row, modal, note, warning};

const LABEL_W: f32 = 110.0;

/// A dialog with a path.
#[derive(Clone, Debug, PartialEq)]
pub enum FileDialog {
    /// A settings file (`.json`) or a full bundle (`.tar`, unpacked into `folder`).
    Import { path: String, folder: String },
    Export {
        name: String,
        full: bool,
        path: String,
    },
    Move {
        name: String,
        current: String,
        folder: String,
    },
    Installers {
        list: Vec<Installer>,
        dir: String,
        /// A delete was asked: read the list again once it is done.
        stale: bool,
    },
    /// A long copy (bundle export or import) with its progress; closes when it ends.
    Busy { title: String, detail: String },
}

impl FileDialog {
    pub fn import() -> Modal {
        Modal::File(Box::new(Self::Import {
            path: String::new(),
            folder: String::new(),
        }))
    }

    pub fn export(v: &VmEntry, home: &str) -> Modal {
        Modal::File(Box::new(Self::Export {
            name: v.display_name().into(),
            full: true,
            path: format!("{home}/{}.tar", slug(v.display_name())),
        }))
    }

    pub fn move_vm(v: &VmEntry, workspace: &str) -> Modal {
        Modal::File(Box::new(Self::Move {
            name: v.display_name().into(),
            current: v.config.workspace.clone(),
            folder: vm_folder(workspace, v.display_name()),
        }))
    }

    pub fn installers(app: &App) -> Modal {
        Modal::File(Box::new(Self::Installers {
            list: app.backend.installers(),
            dir: format!("{}/.installers", app.backend.workspace_root()),
            stale: false,
        }))
    }
}

/// `<Workspace>/<slug of name>`, rg-core's `Registry::vm_folder`.
fn vm_folder(workspace: &str, name: &str) -> String {
    format!("{}/{}", workspace.trim_end_matches('/'), slug(name))
}

fn is_tar(path: &str) -> bool {
    path.trim().to_lowercase().ends_with(".tar")
}

/// The portal's answer: the chosen path, None when cancelled, or why it failed.
type Answer = Receiver<Result<Option<PathBuf>, String>>;

/// Where a portal answer goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PickFor {
    ImportFile,
    ExportFile,
    ExportFolder,
    MoveFolder,
}

/// The file chooser request in flight, and whether the portal answered before.
#[derive(Default)]
pub(crate) struct Picker {
    pending: Option<(PickFor, Answer)>,
    /// Why the portal failed: Browse… is then hidden and the path typed.
    pub unavailable: Option<String>,
}

impl Picker {
    fn busy(&self) -> bool {
        self.pending.is_some()
    }

    /// Asks the portal on its own thread (the request waits for the user).
    fn start(&mut self, ctx: &egui::Context, parent: Option<u32>, what: PickFor, start: &str) {
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        let start = Path::new(start.trim());
        let (folder, name) = if start.is_dir() {
            (start.to_path_buf(), String::new())
        } else {
            (
                start.parent().map(Path::to_path_buf).unwrap_or_default(),
                start
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            )
        };
        let spawned = std::thread::Builder::new()
            .name("file-chooser".into())
            .spawn(move || {
                let r = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt.block_on(portal(what, parent, folder, name)),
                    Err(e) => Err(e.to_string()),
                };
                let _ = tx.send(r);
                ctx.request_repaint();
            });
        match spawned {
            Ok(_) => self.pending = Some((what, rx)),
            Err(e) => self.unavailable = Some(e.to_string()),
        }
    }

    /// The chosen path, once the portal answered.
    fn poll(&mut self) -> Option<(PickFor, PathBuf)> {
        let (what, rx) = self.pending.as_ref()?;
        let what = *what;
        let r = match rx.try_recv() {
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => Err("the file chooser stopped".into()),
            Ok(r) => r,
        };
        self.pending = None;
        match r {
            Ok(p) => p.map(|p| (what, p)),
            Err(e) => {
                self.unavailable = Some(e);
                None
            }
        }
    }
}

async fn portal(
    what: PickFor,
    parent: Option<u32>,
    folder: PathBuf,
    name: String,
) -> Result<Option<PathBuf>, String> {
    let id = parent
        .and_then(|p| format!("x11:{p:x}").parse::<ashpd::WindowIdentifierType>().ok())
        .map(ashpd::WindowIdentifier::from);
    let folder = folder.is_dir().then_some(folder);
    let request = async {
        let files = match what {
            PickFor::ImportFile => {
                SelectedFiles::open_file()
                    .identifier(id)
                    .title("Import VM")
                    .modal(true)
                    .filter(
                        FileFilter::new("VM settings or bundle")
                            .glob("*.json")
                            .glob("*.tar"),
                    )
                    .current_folder::<PathBuf>(folder)?
                    .send()
                    .await?
            }
            PickFor::ExportFile => {
                SelectedFiles::save_file()
                    .identifier(id)
                    .title("Export VM settings")
                    .modal(true)
                    .current_name(name.as_str())
                    .current_folder::<PathBuf>(folder)?
                    .send()
                    .await?
            }
            PickFor::ExportFolder | PickFor::MoveFolder => {
                SelectedFiles::open_file()
                    .identifier(id)
                    .title(if what == PickFor::MoveFolder {
                        "Move VM to"
                    } else {
                        "Export the bundle to"
                    })
                    .modal(true)
                    .directory(true)
                    .current_folder::<PathBuf>(folder)?
                    .send()
                    .await?
            }
        }
        .response()?;
        Ok::<_, ashpd::Error>(files.uris().iter().find_map(|u| uri_path(u.as_str())))
    };
    match request.await {
        Ok(p) => Ok(p),
        Err(ashpd::Error::Response(ashpd::desktop::ResponseError::Cancelled)) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// The local path of a `file://` URI, percent-decoded (rustXterm `platform-linux`, MIT).
fn uri_path(uri: &str) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    let rest = uri.strip_prefix("file://")?;
    let bytes = &rest.as_bytes()[rest.find('/')?..];
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match (bytes[i], hex) {
            (b'%', Some(b)) => {
                out.push(b);
                i += 3;
            }
            (c, _) => {
                out.push(c);
                i += 1;
            }
        }
    }
    Some(PathBuf::from(std::ffi::OsString::from_vec(out)))
}

/// A path field with Browse…; true when Browse… was clicked.
fn path_field(ui: &mut Ui, label: &str, path: &mut String, picker: &Picker, hint: &str) -> bool {
    form_row(ui, LABEL_W, label, |ui| {
        let browse = picker.unavailable.is_none();
        let w = ui.available_width() - if browse { 112.0 } else { 8.0 };
        ui.add(
            egui::TextEdit::singleline(path)
                .hint_text(hint)
                .desired_width(w),
        );
        browse
            && widgets::button(ui, "folder-open", "Browse…", false, !picker.busy())
                .on_hover_text("Choose with the desktop's file chooser")
                .clicked()
    })
}

impl App {
    /// Draws a [`FileDialog`]; `top` is whether it is the top dialog (portal answers go there).
    pub(crate) fn file_dialog(
        &mut self,
        ctx: &egui::Context,
        id: &str,
        d: &mut FileDialog,
        top: bool,
    ) -> Outcome {
        let mut o = Outcome::default();
        if top && let Some((what, p)) = self.picker.poll() {
            let p = p.display().to_string();
            match (what, &mut *d) {
                (PickFor::ImportFile, FileDialog::Import { path, folder }) => {
                    if is_tar(&p) {
                        let stem = Path::new(&p)
                            .file_stem()
                            .map_or("vm".into(), |s| s.to_string_lossy().into_owned());
                        *folder = vm_folder(&self.backend.workspace_root(), &stem);
                    }
                    *path = p;
                }
                (PickFor::ExportFile, FileDialog::Export { path, .. }) => *path = p,
                (PickFor::ExportFolder, FileDialog::Export { path, name, .. }) => {
                    *path = format!("{p}/{}.tar", slug(name));
                }
                (PickFor::MoveFolder, FileDialog::Move { folder, .. }) => *folder = p,
                _ => {}
            }
        }
        let mut browse = None;
        let unavailable = self.picker.unavailable.clone();
        let portal_note = |ui: &mut Ui| {
            if let Some(e) = &unavailable {
                caption(
                    ui,
                    &format!("The desktop's file chooser is not available ({e}): type the path."),
                );
            }
        };
        match d {
            FileDialog::Import { path, folder } => {
                let picker = &self.picker;
                let f = modal(
                    ctx,
                    id,
                    "Import VM",
                    vec2(620.0, 0.0),
                    Some("Import"),
                    Some("Cancel"),
                    |ui| {
                        note(ui, "A settings file (.json) adds the VM with its folder as it was. A full bundle (.tar) is unpacked into a new folder, with its disk, NVRAM, identity and install media.");
                        ui.add_space(6.0);
                        if path_field(ui, "File", path, picker, "/path/to/vm.tar or vm.json") {
                            browse = Some(PickFor::ImportFile);
                        }
                        if is_tar(path) {
                            form_row(ui, LABEL_W, "Folder", |ui| {
                                ui.add(
                                    egui::TextEdit::singleline(folder)
                                        .desired_width(ui.available_width() - 8.0),
                                )
                            });
                        }
                        portal_note(ui);
                    },
                );
                if f == Footer::Ok {
                    let (p, dir) = (path.trim().to_owned(), folder.trim().to_owned());
                    let tar = is_tar(&p);
                    if p.is_empty() || !Path::new(&p).is_file() {
                        o.modals
                            .push(Modal::error("Import VM", "File: choose a .json or .tar file"));
                    } else if tar && dir.is_empty() {
                        o.modals
                            .push(Modal::error("Import VM", "Folder: choose where to unpack it"));
                    } else {
                        if tar {
                            o.modals.push(busy("Importing", &format!("{p} → {dir}")));
                        }
                        o.actions.push(Action::Run(Task::Import {
                            path: p,
                            folder: if tar { dir } else { String::new() },
                        }));
                        o.close = true;
                    }
                } else {
                    o.close = f == Footer::Cancel;
                }
            }
            FileDialog::Export {
                name,
                full,
                path,
            } => {
                let picker = &self.picker;
                let title = format!("Export {name}");
                let f = modal(
                    ctx,
                    id,
                    &title,
                    vec2(620.0, 0.0),
                    Some("Export"),
                    Some("Cancel"),
                    |ui| {
                        let before = *full;
                        ui.radio_value(
                            full,
                            true,
                            "Full VM: settings, disk, NVRAM, identity and install media (.tar)",
                        );
                        ui.radio_value(full, false, "Settings only (.json)");
                        if *full != before {
                            let (from, to) = if *full { (".json", ".tar") } else { (".tar", ".json") };
                            if let Some(stem) = path.strip_suffix(from) {
                                *path = format!("{stem}{to}");
                            }
                        }
                        note(ui, if *full {
                            "The full bundle has everything the VM needs to run on another host; it is about as big as the disk. OpenCore and OVMF are rebuilt by the app at start."
                        } else {
                            "The settings only: import them on another host to add the VM with the same hardware."
                        });
                        ui.add_space(6.0);
                        if path_field(ui, "Save to", path, picker, "") {
                            browse = Some(if *full { PickFor::ExportFolder } else { PickFor::ExportFile });
                        }
                        if Path::new(path.trim()).exists() {
                            warning(ui, "This file exists: it is replaced once the new export is complete.");
                        }
                        portal_note(ui);
                    },
                );
                if f == Footer::Ok {
                    let p = path.trim().to_owned();
                    if p.is_empty() || p.ends_with('/') {
                        o.modals.push(Modal::error("Export VM", "Save to: choose a file"));
                    } else {
                        if *full {
                            o.modals.push(busy(&format!("Exporting {name}"), &p));
                        }
                        o.actions.push(Action::Run(Task::Export { path: p, full: *full }));
                        o.close = true;
                    }
                } else {
                    o.close = f == Footer::Cancel;
                }
            }
            FileDialog::Move {
                name,
                current,
                folder,
            } => {
                let picker = &self.picker;
                let f = modal(
                    ctx,
                    id,
                    &format!("Move {name}"),
                    vec2(620.0, 0.0),
                    Some("Move"),
                    Some("Cancel"),
                    |ui| {
                        form_row(ui, LABEL_W, "Current folder", |ui| ui.label(current.as_str()));
                        if path_field(ui, "New folder", folder, picker, "") {
                            browse = Some(PickFor::MoveFolder);
                        }
                        note(ui, "The disk, firmware variables, boot files and install media move into the new folder (new or empty, on the same file system). If anything fails, everything is put back.");
                        portal_note(ui);
                    },
                );
                if f == Footer::Ok {
                    let dst = folder.trim().to_owned();
                    if dst.is_empty() {
                        o.modals.push(Modal::error("Move VM", "New folder: choose a folder"));
                    } else if dst.trim_end_matches('/') == current.trim_end_matches('/') {
                        o.modals.push(Modal::error("Move VM", "New folder: the VM is already there"));
                    } else {
                        o.actions.push(Action::Run(Task::Move { folder: dst }));
                        o.close = true;
                    }
                } else {
                    o.close = f == Footer::Cancel;
                }
            }
            FileDialog::Installers { list, dir, stale } => {
                if *stale && top && self.busy.is_none() {
                    *list = self.backend.installers();
                    *stale = false;
                }
                let free = self.busy.is_none();
                let mut delete = None;
                let f = modal(ctx, id, "Offline installers", vec2(640.0, 0.0), None, Some("Close"), |ui| {
                    installers_ui(ui, list, dir, free, &mut delete);
                });
                if let Some(i) = delete {
                    *stale = true;
                    o.modals.push(Modal::confirm(
                        "Delete offline installer",
                        &format!(
                            "Delete the macOS {} installer ({:.1} GB)? VMs of this version then install online, or download it again.",
                            i.version, i.size_gb
                        ),
                        Action::Run(Task::RemoveInstaller { build: i.build }),
                    ));
                }
                o.close = f == Footer::Cancel;
            }
            FileDialog::Busy { title, detail } => {
                let progress = self.progress.clone();
                let f = modal(ctx, id, title, vec2(520.0, 0.0), None, Some("Hide"), |ui| {
                    ui.add(egui::Label::new(RichText::new(detail.as_str()).monospace().size(11.5)).wrap());
                    ui.add_space(6.0);
                    let (stage, frac) = progress.unwrap_or_else(|| ("Starting…".into(), None));
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(14.0));
                        ui.label(&stage);
                        if let Some(f) = frac {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.label(RichText::new(format!("{:.0}%", f * 100.0)).color(widgets::tokens(ui).text_muted));
                            });
                        }
                    });
                    widgets::progress_line(ui, egui::Id::new(id).with("bar"), frac.unwrap_or(0.0));
                    caption(ui, "Hide keeps it running; the status bar shows its progress.");
                });
                // It closes on its own when the task ends (a result or an error dialog follows).
                o.close = f == Footer::Cancel || (self.busy.is_none() && !cfg!(test));
            }
        }
        if let Some(what) = browse {
            let start = match &*d {
                FileDialog::Import { path, .. } if !path.trim().is_empty() => path.clone(),
                FileDialog::Export { path, .. } => path.clone(),
                FileDialog::Move { folder, .. } => folder.clone(),
                _ => self.backend.workspace_root(),
            };
            self.picker.start(ctx, self.screen.parent, what, &start);
        }
        o
    }
}

fn busy(title: &str, detail: &str) -> Modal {
    Modal::File(Box::new(FileDialog::Busy {
        title: title.into(),
        detail: detail.into(),
    }))
}

/// One row per cached installer: version, build, size, the VMs using it and Delete.
fn installers_ui(
    ui: &mut Ui,
    list: &[Installer],
    dir: &str,
    free: bool,
    delete: &mut Option<Installer>,
) {
    let t = widgets::tokens(ui);
    let cached: Vec<&Installer> = list.iter().filter(|i| i.cached).collect();
    if cached.is_empty() {
        ui.vertical_centered(|ui| {
            ui.add_space(12.0);
            ui.add(icons::lucide("package", 36.0, t.text_muted));
            ui.label(bold("No offline installers yet"));
        });
        note(
            ui,
            "Download one with Download full installer in a VM's Install step 2 (Big Sur and later); every VM of that macOS version then installs from it with no network.",
        );
    }
    for i in cached {
        widgets::surface(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add(icons::lucide("package", 22.0, t.accent_fg));
                ui.vertical(|ui| {
                    ui.label(bold(format!("macOS {} ({})", i.version, i.build)));
                    let users = if i.used_by.is_empty() {
                        "no VM of this version".to_owned()
                    } else {
                        format!("used by {}", i.used_by.join(", "))
                    };
                    caption(ui, &format!("{:.1} GB · {users}", i.size_gb));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::button(ui, "trash", "Delete", false, free)
                        .on_hover_text("Delete this installer from the cache")
                        .clicked()
                    {
                        *delete = Some(i.clone());
                    }
                });
            });
        });
    }
    ui.add_space(4.0);
    note(
        ui,
        &format!("Kept in {dir}, one per macOS build, shared by every VM of that version."),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_uris_decode() {
        assert_eq!(
            uri_path("file:///home/n/My%20VMs/a.tar"),
            Some(PathBuf::from("/home/n/My VMs/a.tar"))
        );
        assert_eq!(uri_path("https://x/y"), None);
    }

    #[test]
    fn default_folders_use_the_slug() {
        assert_eq!(vm_folder("/w/", "Tahoe lab"), "/w/tahoe-lab");
        assert!(is_tar(" a.TAR ") && !is_tar("a.json"));
    }
}
