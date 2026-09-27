//! Explorer dialogs run on their own STA thread; the renderer keeps responding.
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug)]
pub enum BrowsePurpose {
    Media,
    Shapes,
    Source,
    Destination,
    VideoFrames,
    MinecraftMobs,
}

pub type DialogResult = Result<Vec<PathBuf>, String>;

pub fn open(
    purpose: BrowsePurpose,
    folder: PathBuf,
    owner: usize,
    russian: bool,
) -> std::sync::mpsc::Receiver<DialogResult> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(show(purpose, &folder, owner, russian));
    });
    rx
}

#[cfg(not(windows))]
fn show(_: BrowsePurpose, _: &Path, _: usize, _: bool) -> DialogResult {
    Err(
        "Native dialogs are currently available on Windows. Drag files into the library instead."
            .into(),
    )
}

#[cfg(windows)]
fn show(purpose: BrowsePurpose, folder: &Path, owner: usize, russian: bool) -> DialogResult {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use windows::{
        core::{HSTRING, PCWSTR},
        Win32::{
            Foundation::HWND,
            System::Com::{
                CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize,
                CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
            },
            UI::Shell::{
                Common::COMDLG_FILTERSPEC, FileOpenDialog, IFileOpenDialog, IShellItem,
                SHCreateItemFromParsingName, FOS_ALLOWMULTISELECT, FOS_FILEMUSTEXIST,
                FOS_FORCEFILESYSTEM, FOS_PATHMUSTEXIST, FOS_PICKFOLDERS, SIGDN_FILESYSPATH,
            },
        },
    };
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe { CoUninitialize() }
        }
    }
    let run = || -> windows::core::Result<Vec<PathBuf>> {
        // All COM objects are created, used and released on this STA thread.
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
            let _apartment = Apartment;
            let dialog: IFileOpenDialog =
                CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
            let is_folder = matches!(
                purpose,
                BrowsePurpose::Source | BrowsePurpose::Destination | BrowsePurpose::MinecraftMobs
            );
            let options = FOS_FORCEFILESYSTEM
                | FOS_PATHMUSTEXIST
                | if is_folder {
                    FOS_PICKFOLDERS
                } else if matches!(purpose, BrowsePurpose::VideoFrames) {
                    FOS_FILEMUSTEXIST
                } else {
                    FOS_ALLOWMULTISELECT | FOS_FILEMUSTEXIST
                };
            dialog.SetOptions(dialog.GetOptions()? | options)?;
            let title = match (purpose, russian) {
                (BrowsePurpose::Media, true) => "Что собираем — выберите фото или видео",
                (BrowsePurpose::Media, false) => "Target — choose images or videos",
                (BrowsePurpose::Shapes, true) => "Из каких картинок — выберите фотографии",
                (BrowsePurpose::Shapes, false) => "Building blocks — choose photos",
                (BrowsePurpose::Source, true) => "Выберите папку исходных изображений",
                (BrowsePurpose::Source, false) => "Choose the source folder",
                (BrowsePurpose::Destination, true) => "Выберите папку для результата конвертации",
                (BrowsePurpose::Destination, false) => "Choose the conversion output folder",
                (BrowsePurpose::VideoFrames, true) => "Выберите видео для создания набора картинок",
                (BrowsePurpose::VideoFrames, false) => "Choose a video to create a shape set",
                (BrowsePurpose::MinecraftMobs, true) => "Выберите набор мобов с mob_mapping.json",
                (BrowsePurpose::MinecraftMobs, false) => {
                    "Choose a mob export containing mob_mapping.json"
                }
            };
            dialog.SetTitle(&HSTRING::from(title))?;
            if !is_folder {
                let mut extensions = crate::io::media_loader::SUPPORTED_IMAGE_EXTENSIONS.to_vec();
                if matches!(purpose, BrowsePurpose::VideoFrames) {
                    extensions = crate::io::media_loader::SUPPORTED_VIDEO_EXTENSIONS.to_vec();
                }
                if matches!(purpose, BrowsePurpose::Media) {
                    extensions.extend(crate::io::media_loader::SUPPORTED_VIDEO_EXTENSIONS);
                }
                let pattern = HSTRING::from(
                    extensions
                        .iter()
                        .map(|e| format!("*.{e}"))
                        .collect::<Vec<_>>()
                        .join(";"),
                );
                let name = HSTRING::from(if russian {
                    "Изображения / медиа"
                } else {
                    "Images / media"
                });
                dialog.SetFileTypes(&[COMDLG_FILTERSPEC {
                    pszName: PCWSTR(name.as_ptr()),
                    pszSpec: PCWSTR(pattern.as_ptr()),
                }])?;
            }
            if folder.is_dir() {
                let wide: Vec<u16> = folder.as_os_str().encode_wide().chain(Some(0)).collect();
                if let Ok(item) =
                    SHCreateItemFromParsingName::<_, _, IShellItem>(PCWSTR(wide.as_ptr()), None)
                {
                    let _ = dialog.SetFolder(&item);
                }
            }
            match dialog.Show(HWND(owner as *mut std::ffi::c_void)) {
                Ok(()) => {}
                Err(e) if e.code().0 as u32 == 0x800704c7 => return Ok(Vec::new()),
                Err(e) => return Err(e),
            }
            let items = dialog.GetResults()?;
            let mut paths = Vec::new();
            for index in 0..items.GetCount()? {
                let item = items.GetItemAt(index)?;
                let name = item.GetDisplayName(SIGDN_FILESYSPATH)?;
                paths.push(PathBuf::from(std::ffi::OsString::from_wide(name.as_wide())));
                CoTaskMemFree(Some(name.0.cast()));
            }
            Ok(paths)
        }
    };
    run().map_err(|e| format!("Windows: {e}"))
}
