//! 自定义操作：打开网址 / 运行 Shell 命令，以及「一键检测是否可用」

use crate::config::CustomAction;
use serde::Serialize;
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// 传给操作的输入
pub struct Input {
    pub kind: &'static str,
    /// 文本内容；图片为 OCR 文字（如已识别）；文件为换行分隔的路径
    pub text: String,
    /// 图片文件路径或第一个文件的路径
    pub file: Option<String>,
}

pub enum Outcome {
    /// 已打开网址或命令无需处理输出
    Done,
    /// 命令输出，按 action.output 处理
    Output(String),
}

const RUN_TIMEOUT: Duration = Duration::from_secs(15);
const TEST_TIMEOUT: Duration = Duration::from_secs(6);

pub fn run(action: &CustomAction, input: &Input) -> Result<Outcome, String> {
    match action.kind.as_str() {
        "url" => {
            let url = fill_url(&action.template, input);
            spawn_detached("open", &[&url])?;
            Ok(Outcome::Done)
        }
        "shell" => {
            let out = run_shell(&action.template, input, RUN_TIMEOUT)?;
            if !out.success {
                return Err(format!("命令执行失败（退出码 {}）：{}", out.code, out.stderr.trim()));
            }
            Ok(Outcome::Output(out.stdout.trim_end().to_string()))
        }
        other => Err(format!("未知操作类型 {other}")),
    }
}

#[derive(Serialize)]
pub struct TestResult {
    pub ok: bool,
    pub message: String,
}

/// 一键检测：网址检查是否有应用能打开、http(s) 再检查是否可访问；命令用示例输入试运行
pub fn test(action: &CustomAction) -> TestResult {
    let sample = Input {
        kind: "text",
        text: "Hello wtools 你好".into(),
        file: None,
    };
    let fail = |m: String| TestResult { ok: false, message: m };
    match action.kind.as_str() {
        "url" => {
            if action.template.trim().is_empty() {
                return fail("网址不能为空".into());
            }
            let url = fill_url(&action.template, &sample);
            let Some(scheme) = url.split_once(':').map(|(s, _)| s.to_lowercase()) else {
                return fail("网址格式不正确，缺少协议（如 https://）".into());
            };
            if !has_handler(&url) {
                return fail(format!("系统中没有能打开 {scheme}: 链接的应用"));
            }
            if scheme != "http" && scheme != "https" {
                return TestResult { ok: true, message: format!("可用：系统中有应用能打开 {scheme}: 链接") };
            }
            match http_status(&url) {
                Ok(code) if (200..400).contains(&code) => TestResult { ok: true, message: format!("可用：网址可访问（HTTP {code}）") },
                Ok(code) => fail(format!("网址返回 HTTP {code}，请检查模板")),
                Err(e) => fail(format!("无法访问：{e}")),
            }
        }
        "shell" => {
            if action.template.trim().is_empty() {
                return fail("命令不能为空".into());
            }
            match run_shell(&action.template, &sample, TEST_TIMEOUT) {
                Ok(o) if o.success => {
                    let preview: String = o.stdout.trim().chars().take(80).collect();
                    TestResult {
                        ok: true,
                        message: if preview.is_empty() {
                            "可用：命令执行成功（无输出）".into()
                        } else {
                            format!("可用：示例输出「{preview}」")
                        },
                    }
                }
                Ok(o) => fail(format!("退出码 {}：{}", o.code, o.stderr.trim().chars().take(120).collect::<String>())),
                Err(e) => fail(e),
            }
        }
        other => fail(format!("未知操作类型 {other}")),
    }
}

fn fill_url(template: &str, input: &Input) -> String {
    template
        .replace("{text}", &url_encode(&input.text))
        .replace("{file}", &url_encode(input.file.as_deref().unwrap_or("")))
}

struct ShellOutput {
    success: bool,
    code: i32,
    stdout: String,
    stderr: String,
}

fn run_shell(script: &str, input: &Input, timeout: Duration) -> Result<ShellOutput, String> {
    // GUI 应用的 PATH 很短，补上 Homebrew 等常见目录
    let path = format!(
        "/opt/homebrew/bin:/usr/local/bin:{}",
        std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin:/usr/sbin:/sbin".into())
    );
    let mut child = Command::new("/bin/sh")
        .arg("-c")
        .arg(script)
        .env("PATH", path)
        .env("WTOOLS_TEXT", &input.text)
        .env("WTOOLS_FILE", input.file.as_deref().unwrap_or(""))
        .env("WTOOLS_KIND", input.kind)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("无法启动命令: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        let text = input.text.clone();
        // 单独线程写入，避免输出缓冲区满时互相阻塞
        std::thread::spawn(move || {
            let _ = stdin.write_all(text.as_bytes());
        });
    }
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("命令超时（{} 秒）", timeout.as_secs()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => return Err(e.to_string()),
        }
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    Ok(ShellOutput {
        success: out.status.success(),
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    })
}

fn http_status(url: &str) -> Result<u16, String> {
    let out = Command::new("curl")
        .args(["-sS", "-o", "/dev/null", "-L", "--max-time", "6", "-w", "%{http_code}", url])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .map_err(|_| "无法解析响应".into())
}

#[cfg(target_os = "macos")]
fn has_handler(url: &str) -> bool {
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::{NSString, NSURL};
    objc2::rc::autoreleasepool(|_| {
        NSURL::URLWithString(&NSString::from_str(url))
            .and_then(|u| NSWorkspace::sharedWorkspace().URLForApplicationToOpenURL(&u))
            .is_some()
    })
}

#[cfg(not(target_os = "macos"))]
fn has_handler(_url: &str) -> bool {
    true
}

fn spawn_detached(program: &str, args: &[&str]) -> Result<(), String> {
    let mut child = Command::new(program)
        .args(args)
        .spawn()
        .map_err(|e| e.to_string())?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action(kind: &str, template: &str) -> CustomAction {
        CustomAction {
            id: "t".into(),
            name: "t".into(),
            kind: kind.into(),
            template: template.into(),
            applies_to: vec!["text".into()],
            output: "show".into(),
        }
    }

    #[test]
    fn url_template_is_encoded() {
        let input = Input { kind: "text", text: "a b&c 中".into(), file: None };
        assert_eq!(fill_url("https://x.com/?q={text}", &input), "https://x.com/?q=a%20b%26c%20%E4%B8%AD");
    }

    #[test]
    fn shell_receives_stdin_and_env() {
        let input = Input { kind: "text", text: "hello".into(), file: None };
        let out = run_shell("tr a-z A-Z; printf ' %s' \"$WTOOLS_KIND\"", &input, TEST_TIMEOUT).unwrap();
        assert!(out.success);
        assert_eq!(out.stdout, "HELLO text");
    }

    #[test]
    fn shell_test_reports_failure_and_timeout() {
        assert!(!test(&action("shell", "exit 3")).ok);
        assert!(test(&action("shell", "cat")).ok);
        let start = Instant::now();
        let r = run_shell("sleep 5", &Input { kind: "text", text: String::new(), file: None }, Duration::from_millis(300));
        assert!(r.is_err() && start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn url_without_scheme_fails() {
        assert!(!test(&action("url", "example.com/{text}")).ok);
    }
}
