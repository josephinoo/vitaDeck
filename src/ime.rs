pub enum ImeResult {
    Confirmed(String),
    Canceled,
}

#[cfg(target_os = "vita")]
const PSP2_SDK_VERSION: u32 = 0x0357_0011;

pub struct ImeDialog {
    active: bool,
    buffer: Vec<u16>,
    #[allow(dead_code)]
    title: Vec<u16>,
}

impl Default for ImeDialog {
    fn default() -> Self {
        Self::new()
    }
}

impl ImeDialog {
    pub fn new() -> Self {
        Self {
            active: false,
            buffer: Vec::new(),
            title: Vec::new(),
        }
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn open(&mut self, title: &str, initial_text: &str, max_len: usize) -> bool {
        if self.active {
            return false;
        }

        self.title = to_utf16_null_terminated(title);
        self.buffer = vec![0u16; max_len + 1];
        for (slot, ch) in self.buffer.iter_mut().zip(initial_text.encode_utf16()) {
            *slot = ch;
        }
        *self.buffer.last_mut().unwrap() = 0;

        #[cfg(target_os = "vita")]
        {
            unsafe {
                let mut param: vitasdk_sys::SceImeDialogParam = core::mem::zeroed();

                let common_ptr = &raw mut param.commonParam;
                param.commonParam.magic = vitasdk_sys::SCE_COMMON_DIALOG_MAGIC_NUMBER
                    .wrapping_add(common_ptr as usize as u32);
                param.sdkVersion = PSP2_SDK_VERSION;

                param.title = self.title.as_ptr();
                param.maxTextLength = max_len as u32;
                param.initialText = self.buffer.as_mut_ptr();
                param.inputTextBuffer = self.buffer.as_mut_ptr();
                param.type_ = vitasdk_sys::SCE_IME_TYPE_BASIC_LATIN as u32;
                param.supportedLanguages = (vitasdk_sys::SCE_IME_LANGUAGE_ENGLISH
                    | vitasdk_sys::SCE_IME_LANGUAGE_SPANISH) as u64;
                param.languagesForced = 0;
                param.dialogMode = vitasdk_sys::SCE_IME_DIALOG_DIALOG_MODE_WITH_CANCEL;
                param.textBoxMode = vitasdk_sys::SCE_IME_DIALOG_TEXTBOX_MODE_WITH_CLEAR;
                param.enterLabel = vitasdk_sys::SCE_IME_ENTER_LABEL_SEARCH as u8;

                let res = vitasdk_sys::sceImeDialogInit(&param);
                if res < 0 {
                    crate::logger::log(&format!("ImeDialog: sceImeDialogInit failed: {:#X}", res));
                    return false;
                }
            }
        }

        self.active = true;
        crate::logger::log(&format!("ImeDialog: opened for \"{}\"", title));
        true
    }

    pub fn poll(&mut self) -> Option<ImeResult> {
        if !self.active {
            return None;
        }

        #[cfg(target_os = "vita")]
        {
            unsafe {
                let status = vitasdk_sys::sceImeDialogGetStatus();
                match status as u32 {
                    vitasdk_sys::SCE_COMMON_DIALOG_STATUS_RUNNING => {
                        return None;
                    }
                    vitasdk_sys::SCE_COMMON_DIALOG_STATUS_FINISHED => {
                        let mut result: vitasdk_sys::SceImeDialogResult = core::mem::zeroed();
                        let res = vitasdk_sys::sceImeDialogGetResult(&mut result);
                        let _ = vitasdk_sys::sceImeDialogTerm();
                        self.active = false;

                        if res >= 0 && result.button as u32 == vitasdk_sys::SCE_IME_DIALOG_BUTTON_ENTER as u32 {
                            let text = from_utf16_null_terminated(&self.buffer);
                            crate::logger::log(&format!("ImeDialog: confirmed with text: \"{}\"", text));
                            return Some(ImeResult::Confirmed(text));
                        } else {
                            crate::logger::log("ImeDialog: canceled / closed");
                            return Some(ImeResult::Canceled);
                        }
                    }
                    _ => {
                        let _ = vitasdk_sys::sceImeDialogTerm();
                        self.active = false;
                        return Some(ImeResult::Canceled);
                    }
                }
            }
        }

        #[cfg(not(target_os = "vita"))]
        {
            self.active = false;
            None
        }
    }
}

fn to_utf16_null_terminated(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(target_os = "vita")]
fn from_utf16_null_terminated(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}
