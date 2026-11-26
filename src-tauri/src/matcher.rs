//! 搜索匹配：模糊匹配 + 拼音（全拼 / 首字母）

use nucleo_matcher::{
    pattern::{CaseMatching, Normalization, Pattern},
    Config, Matcher, Utf32Str,
};
use pinyin::ToPinyin;

/// 一个可被搜索的条目预先算好的匹配串
/// `primary` 来自标题本身（含拼音、缩写），`secondary` 来自附加关键词；
/// 关键词命中的得分打折，避免「玩游戏」这类附加词让 App Store 排在真正匹配的应用前面。
#[derive(Debug, Clone)]
pub struct Haystack {
    primary: Vec<String>,
    secondary: Vec<String>,
}

impl Haystack {
    pub fn new(title: &str, keywords: &[&str]) -> Self {
        let primary = variants(title);
        let mut secondary: Vec<String> = Vec::new();
        for kw in keywords {
            for v in variants(kw) {
                if !primary.contains(&v) && !secondary.contains(&v) {
                    secondary.push(v);
                }
            }
        }
        Self { primary, secondary }
    }
}

/// 一段文本的所有匹配形式：原文、全拼、拼音首字母、英文单词首字母
fn variants(text: &str) -> Vec<String> {
    let mut v = vec![text.to_string()];
    if let Some((full, initials)) = to_pinyin(text) {
        v.push(full);
        v.push(initials);
    }
    if let Some(acronym) = word_initials(text) {
        v.push(acronym);
    }
    v.dedup();
    v
}

/// 多个英文单词取首字母：Visual Studio Code → vsc；单个单词返回 None
fn word_initials(text: &str) -> Option<String> {
    let words: Vec<&str> = text
        .split(|c: char| c.is_whitespace() || c == '-' || c == '_' || c == '.')
        .filter(|w| w.chars().next().is_some_and(|c| c.is_ascii_alphanumeric()))
        .collect();
    (words.len() >= 2).then(|| {
        words
            .iter()
            .filter_map(|w| w.chars().next())
            .map(|c| c.to_ascii_lowercase())
            .collect()
    })
}

/// 中文转「全拼」和「首字母」；不含中文时返回 None
fn to_pinyin(text: &str) -> Option<(String, String)> {
    let mut full = String::new();
    let mut initials = String::new();
    let mut has_cn = false;
    for (ch, py) in text.chars().zip(text.to_pinyin()) {
        match py {
            Some(p) => {
                has_cn = true;
                full.push_str(p.plain());
                initials.push_str(p.first_letter());
            }
            None if ch.is_alphanumeric() => {
                full.push(ch);
                initials.push(ch);
            }
            None => {}
        }
    }
    has_cn.then_some((full, initials))
}

pub struct Query {
    raw: String,
    pattern: Pattern,
}

impl Query {
    pub fn new(raw: &str) -> Self {
        Self {
            raw: raw.trim().to_lowercase(),
            pattern: Pattern::parse(raw.trim(), CaseMatching::Ignore, Normalization::Smart),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.raw.is_empty()
    }
}

pub struct FuzzyMatcher {
    matcher: Matcher,
    buf: Vec<char>,
}

impl FuzzyMatcher {
    pub fn new() -> Self {
        Self {
            matcher: Matcher::new(Config::DEFAULT),
            buf: Vec::new(),
        }
    }

    /// 返回最高分；完全不匹配返回 None。
    /// 在模糊分之上按命中方式加分：完全相同 > 前缀 > 连续子串（如 "wx" 命中 "qywx"）。
    pub fn score(&mut self, query: &Query, hay: &Haystack) -> Option<u32> {
        let primary = hay.primary.iter().filter_map(|c| self.score_one(query, c)).max();
        let secondary = hay
            .secondary
            .iter()
            .filter_map(|c| self.score_one(query, c))
            .max()
            .map(|s| s * 3 / 5);
        primary.max(secondary)
    }

    fn score_one(&mut self, query: &Query, cand: &str) -> Option<u32> {
        let s = query
            .pattern
            .score(Utf32Str::new(cand, &mut self.buf), &mut self.matcher)?;
        let lower = cand.to_lowercase();
        let bonus = if lower == query.raw {
            1000
        } else if lower.starts_with(&query.raw) {
            500
        } else if lower.contains(&query.raw) {
            300
        } else {
            0
        };
        Some(s + bonus)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn score(q: &str, title: &str, kw: &[&str]) -> Option<u32> {
        FuzzyMatcher::new().score(&Query::new(q), &Haystack::new(title, kw))
    }

    #[test]
    fn matches_pinyin_initials_and_full() {
        assert!(score("sp", "锁屏", &[]).is_some());
        assert!(score("suoping", "锁屏", &[]).is_some());
        assert!(score("锁", "锁屏", &[]).is_some());
        assert!(score("lock", "锁屏", &["lock"]).is_some());
        assert!(score("xyz", "锁屏", &["lock"]).is_none());
    }

    #[test]
    fn matches_english_acronym() {
        assert!(score("vsc", "Visual Studio Code", &[]).is_some());
        assert!(score("wx", "微信", &["WeChat"]).is_some());
        assert!(score("wechat", "微信", &["WeChat"]).is_some());
    }

    #[test]
    fn title_beats_keyword() {
        let title = score("wx", "企业微信", &[]).unwrap();
        let kw = score("wx", "App Store", &["玩游戏"]).unwrap();
        assert!(title > kw, "{title} vs {kw}");
    }

    #[test]
    fn prefix_ranks_higher() {
        let prefix = score("sp", "锁屏", &[]).unwrap();
        let fuzzy = score("sp", "清空废纸篓", &["empty trash"]);
        assert!(fuzzy.is_none_or(|f| prefix > f));
    }
}
