//! 文字识别（OCR）：系统 Vision 框架，离线、支持中英文
//!
//! 识别结果按行返回，每行再拆成「词块」供界面点选：
//! - 中文用系统分词（NaturalLanguage 的 NLTokenizer）切成词
//! - 网址、邮箱、电话、金额、日期时间等先整体识别出来，保持完整不拆开
//! - 标点单独成块，保留「前面是否有空格」，复制时能还原原文的空格

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Token {
    pub text: String,
    /// 原文中该词前面有空白
    pub space: bool,
    /// 网址 / 电话 / 金额等整体识别的内容
    #[serde(default)]
    pub atomic: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Line {
    pub text: String,
    pub tokens: Vec<Token>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OcrResult {
    pub lines: Vec<Line>,
}

impl OcrResult {
    pub fn text(&self) -> String {
        self.lines
            .iter()
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

pub const CLI_FLAG: &str = "--ocr";

/// 在独立子进程中识别图片文件。
/// Vision 会把识别模型常驻内存（约 40 MB），放到用完即退出的子进程里，主进程内存不受影响。
pub fn recognize_file(path: &std::path::Path) -> Result<OcrResult, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let out = std::process::Command::new(exe)
        .arg(CLI_FLAG)
        .arg(path)
        .output()
        .map_err(|e| format!("启动识别进程失败: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("解析识别结果失败: {e}"))
}

/// 子进程入口：读取图片、识别并返回 JSON
pub fn cli(path: &str) -> Result<String, String> {
    let png = std::fs::read(path).map_err(|e| format!("读取图片失败: {e}"))?;
    let result = recognize(&png)?;
    serde_json::to_string(&result).map_err(|e| e.to_string())
}

#[cfg(target_os = "macos")]
pub fn recognize(png: &[u8]) -> Result<OcrResult, String> {
    use objc2::rc::{autoreleasepool, Retained};
    use objc2::AnyThread;
    use objc2_foundation::{NSArray, NSData, NSDictionary, NSString};
    use objc2_vision::{
        VNImageRequestHandler, VNRecognizeTextRequest, VNRequest, VNRequestTextRecognitionLevel,
    };

    autoreleasepool(|_| {
        let data = NSData::with_bytes(png);
        let handler = VNImageRequestHandler::initWithData_options(
            VNImageRequestHandler::alloc(),
            &data,
            &NSDictionary::new(),
        );
        let request = VNRecognizeTextRequest::new();
        request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
        request.setUsesLanguageCorrection(true);
        let langs: Vec<Retained<NSString>> = ["zh-Hans", "zh-Hant", "en-US"]
            .iter()
            .map(|l| NSString::from_str(l))
            .collect();
        request.setRecognitionLanguages(&NSArray::from_retained_slice(&langs));

        let as_request: Retained<VNRequest> = Retained::into_super(Retained::into_super(request.clone()));
        handler
            .performRequests_error(&NSArray::from_retained_slice(&[as_request]))
            .map_err(|e| format!("识别失败: {}", e.localizedDescription()))?;

        let splitter = Splitter::new();
        let mut lines = Vec::new();
        if let Some(results) = request.results() {
            for obs in results.iter() {
                let Some(best) = obs.topCandidates(1).firstObject() else { continue };
                let text = best.string().to_string();
                if text.trim().is_empty() {
                    continue;
                }
                lines.push(Line {
                    tokens: splitter.split(&text),
                    text,
                });
            }
        }
        Ok(OcrResult { lines })
    })
}

#[cfg(not(target_os = "macos"))]
pub fn recognize(_png: &[u8]) -> Result<OcrResult, String> {
    Err("当前系统不支持 OCR".into())
}

/// 把一行文字拆成词块
#[cfg(target_os = "macos")]
struct Splitter {
    detector: Option<objc2::rc::Retained<objc2_foundation::NSDataDetector>>,
    patterns: Vec<objc2::rc::Retained<objc2_foundation::NSRegularExpression>>,
}

#[cfg(target_os = "macos")]
impl Splitter {
    fn new() -> Self {
        use objc2_foundation::{NSDataDetector, NSRegularExpression, NSRegularExpressionOptions, NSString};
        const LINK: u64 = 1 << 5;
        const PHONE: u64 = 1 << 11;
        // 需要整体保留的内容：邮箱、金额/数字（含千分位、小数、百分号、货币单位）、日期、时间
        const PATTERNS: &[&str] = &[
            r"[\w.+-]+@[\w-]+(?:\.[\w-]+)+",
            r"\d{4}[-/.年]\d{1,2}[-/.月]\d{1,2}日?",
            r"\d{1,2}:\d{2}(?::\d{2})?",
            r"[¥￥$€£]?\s?[+-]?\d[\d,]*(?:\.\d+)?\s?(?:%|‰|元|万元|亿元|万|亿|美元|块|USD|CNY|RMB)?",
        ];
        Self {
            detector: NSDataDetector::dataDetectorWithTypes_error(LINK | PHONE).ok(),
            patterns: PATTERNS
                .iter()
                .filter_map(|p| {
                    NSRegularExpression::regularExpressionWithPattern_options_error(
                        &NSString::from_str(p),
                        NSRegularExpressionOptions(0),
                    )
                    .ok()
                })
                .collect(),
        }
    }

    fn split(&self, text: &str) -> Vec<Token> {
        use objc2_foundation::{NSMatchingOptions, NSRange, NSString};
        use objc2_natural_language::{NLTokenUnit, NLTokenizer};
        use objc2::AnyThread;

        let ns = NSString::from_str(text);
        let units: Vec<u16> = text.encode_utf16().collect();
        let full = NSRange::new(0, units.len());

        // 1. 找出需要整体保留的区间（UTF-16 下标），重叠的合并
        let mut atomic: Vec<(usize, usize)> = Vec::new();
        let mut collect = |matches: objc2::rc::Retained<objc2_foundation::NSArray<objc2_foundation::NSTextCheckingResult>>| {
            for m in matches.iter() {
                let r = m.range();
                let (s, e) = (r.location, r.location + r.length);
                // 纯标点/空白的匹配（如单独的 "$"）没有意义
                if units[s..e].iter().any(|c| char::from_u32(*c as u32).is_some_and(|c| c.is_alphanumeric())) {
                    atomic.push((s, e));
                }
            }
        };
        if let Some(d) = &self.detector {
            collect(d.matchesInString_options_range(&ns, NSMatchingOptions(0), full));
        }
        for p in &self.patterns {
            collect(p.matchesInString_options_range(&ns, NSMatchingOptions(0), full));
        }
        atomic.sort();
        let mut merged: Vec<(usize, usize)> = Vec::new();
        for (s, e) in atomic {
            match merged.last_mut() {
                Some(last) if s < last.1 => last.1 = last.1.max(e),
                _ => merged.push((s, e)),
            }
        }

        // 2. 区间之外的部分用系统分词
        let tokenizer = unsafe { NLTokenizer::initWithUnit(NLTokenizer::alloc(), NLTokenUnit::Word) };
        unsafe { tokenizer.setString(Some(&ns)) };
        let mut words: Vec<(usize, usize, bool)> = Vec::new(); // (start, end, atomic)
        let mut cursor = 0;
        for &(s, e) in merged.iter().chain(std::iter::once(&(units.len(), units.len()))) {
            if cursor < s {
                let gap = NSRange::new(cursor, s - cursor);
                for v in unsafe { tokenizer.tokensForRange(gap) }.iter() {
                    if let Some(r) = v.get_range() {
                        let (ts, te) = (r.location.max(cursor), (r.location + r.length).min(s));
                        if ts < te {
                            words.push((ts, te, false));
                        }
                    }
                }
            }
            if s < e {
                words.push((s, e, true));
            }
            cursor = cursor.max(e);
        }

        // 3. 词与词之间剩下的字符：空白记为下一个词的前导空格，标点各自成块
        let mut tokens = Vec::new();
        let mut pos = 0;
        let mut pending_space = false;
        let flush_gap = |from: usize, to: usize, tokens: &mut Vec<Token>, pending_space: &mut bool| {
            for ch in String::from_utf16_lossy(&units[from..to]).chars() {
                if ch.is_whitespace() {
                    *pending_space = true;
                } else {
                    tokens.push(Token { text: ch.to_string(), space: *pending_space, atomic: false });
                    *pending_space = false;
                }
            }
        };
        for (s, e, is_atomic) in words {
            if s < pos {
                continue;
            }
            flush_gap(pos, s, &mut tokens, &mut pending_space);
            let text = String::from_utf16_lossy(&units[s..e]);
            tokens.push(Token { text: text.trim().to_string(), space: pending_space || text.starts_with(char::is_whitespace), atomic: is_atomic });
            pending_space = false;
            pos = e;
        }
        flush_gap(pos, units.len(), &mut tokens, &mut pending_space);
        tokens.retain(|t| !t.text.is_empty());
        tokens
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    fn texts(line: &str) -> Vec<String> {
        Splitter::new().split(line).into_iter().map(|t| t.text).collect()
    }

    #[test]
    fn keeps_url_phone_amount_whole() {
        let t = texts("访问 https://example.com/a?b=1 或致电 138-0013-8000，金额 ¥1,234.56 元");
        assert!(t.contains(&"https://example.com/a?b=1".to_string()), "{t:?}");
        assert!(t.iter().any(|x| x.contains("138-0013-8000")), "{t:?}");
        assert!(t.iter().any(|x| x.starts_with("¥1,234.56")), "{t:?}");
    }

    #[test]
    fn splits_chinese_words_and_punctuation() {
        let t = texts("今天天气很好，我们去公园。");
        assert!(t.len() >= 5, "{t:?}");
        assert!(t.contains(&"，".to_string()));
        assert_eq!(t.concat(), "今天天气很好，我们去公园。");
    }

    #[test]
    fn records_spaces_between_english_words() {
        let tokens = Splitter::new().split("Hello world, wtools!");
        let joined: String = tokens
            .iter()
            .map(|t| format!("{}{}", if t.space { " " } else { "" }, t.text))
            .collect();
        assert_eq!(joined.trim(), "Hello world, wtools!");
    }

    /// 本机真实识别：`cargo test real_ocr -- --ignored --nocapture`（需要 $WTOOLS_OCR_IMG）
    #[test]
    #[ignore]
    fn real_ocr() {
        let path = std::env::var("WTOOLS_OCR_IMG").expect("set WTOOLS_OCR_IMG");
        let t = std::time::Instant::now();
        let r = recognize(&std::fs::read(path).unwrap()).unwrap();
        println!("耗时 {:?}", t.elapsed());
        for l in &r.lines {
            let toks: Vec<_> = l.tokens.iter().map(|t| t.text.as_str()).collect();
            println!("{} => {:?}", l.text, toks);
        }
    }
}
