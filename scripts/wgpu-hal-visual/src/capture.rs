//! Test-only Win32 capture of the harness's own HWND. No desktop or other-app capture.
use std::{path::PathBuf, sync::mpsc};
use windows::Win32::{
    Foundation::{HWND, POINT, RECT},
    Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, ClientToScreen, CreateCompatibleBitmap, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject,
        GetDIBits, GetWindowDC, HBITMAP, HDC, HGDIOBJ, ReleaseDC, SelectObject,
    },
    Storage::Xps::{PRINT_WINDOW_FLAGS, PrintWindow},
    UI::WindowsAndMessaging::{GetClientRect, GetWindowRect, GetWindowThreadProcessId},
};

pub struct Info {
    pub window_size: [u32; 2],
    pub client_size: [u32; 2],
    pub client_offset: [i32; 2],
}

// These handles stay on the capture worker; the application keeps its window
// alive until the worker replies. Drop releases them on every error path.
struct Resources {
    hwnd: HWND,
    dc: HDC,
    memory: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
}

impl Drop for Resources {
    fn drop(&mut self) {
        // SAFETY: all handles were created/acquired below on this thread.
        unsafe {
            SelectObject(self.memory, self.previous);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.memory);
            ReleaseDC(Some(self.hwnd), self.dc);
        }
    }
}

fn capture(value: isize, path: &std::path::Path) -> Result<Info, String> {
    let hwnd = HWND(value as *mut _);
    // SAFETY: the main event loop supplied its own HWND. Ownership is checked
    // before capture; Windows rejects a handle if the user closes the window.
    unsafe {
        let mut process = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut process));
        if process != std::process::id() {
            return Err("Capture target is not our window".into());
        }
        let mut bounds = RECT::default();
        let mut client = RECT::default();
        let mut origin = POINT::default();
        GetWindowRect(hwnd, &mut bounds).map_err(|e| e.to_string())?;
        GetClientRect(hwnd, &mut client).map_err(|e| e.to_string())?;
        ClientToScreen(hwnd, &mut origin).ok().map_err(|e| e.to_string())?;
        let width = bounds.right.checked_sub(bounds.left).ok_or("Invalid window width")?;
        let height = bounds.bottom.checked_sub(bounds.top).ok_or("Invalid window height")?;
        if width <= 0 || height <= 0 || i64::from(width) * i64::from(height) > 32_000_000 {
            return Err("Invalid or excessive capture size".into());
        }
        let dc = GetWindowDC(Some(hwnd));
        if dc.0.is_null() {
            return Err("GetWindowDC failed".into());
        }
        let memory = CreateCompatibleDC(Some(dc));
        if memory.0.is_null() {
            ReleaseDC(Some(hwnd), dc);
            return Err("CreateCompatibleDC failed".into());
        }
        let bitmap = CreateCompatibleBitmap(dc, width, height);
        if bitmap.0.is_null() {
            let _ = DeleteDC(memory);
            ReleaseDC(Some(hwnd), dc);
            return Err("CreateCompatibleBitmap failed".into());
        }
        let previous = SelectObject(memory, bitmap.into());
        let resources = Resources { hwnd, dc, memory, bitmap, previous };
        // PW_RENDERFULLCONTENT keeps the capture in the real window's pixel
        // coordinates. Cropping/offset handling happens in the comparison.
        PrintWindow(hwnd, resources.memory, PRINT_WINDOW_FLAGS(2)).ok().map_err(|e| e.to_string())?;
        SelectObject(resources.memory, resources.previous);
        let mut info = BITMAPINFO::default();
        info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = width;
        info.bmiHeader.biHeight = -height;
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB.0;
        let mut bytes = vec![0u8; (width * height * 4) as usize];
        if GetDIBits(resources.memory, resources.bitmap, 0, height as u32, Some(bytes.as_mut_ptr().cast()), &mut info, DIB_RGB_COLORS) != height {
            return Err("GetDIBits did not return the entire image".into());
        }
        let mut image = image::RgbImage::new(width as u32, height as u32);
        for (pixel, data) in image.pixels_mut().zip(bytes.as_chunks::<4>().0) {
            *pixel = image::Rgb([data[2], data[1], data[0]]);
        }
        image.save(path).map_err(|e| e.to_string())?;
        Ok(Info {
            window_size: [width as u32, height as u32],
            client_size: [(client.right - client.left) as u32, (client.bottom - client.top) as u32],
            client_offset: [origin.x - bounds.left, origin.y - bounds.top],
        })
    }
}

pub fn spawn(value: isize, path: PathBuf) -> mpsc::Receiver<Result<Info, String>> {
    let (sender, receiver) = mpsc::channel();
    // PrintWindow may send WM_PRINT. Run it off the event-loop thread so that
    // the test cannot deadlock its own window while waiting for the result.
    // A spawn failure disconnects the channel, which the event loop reports.
    let _ = std::thread::Builder::new().name("window-capture".into()).spawn(move || {
        let _ = sender.send(capture(value, &path));
    });
    receiver
}
