//#region 비상 해제 문구 입력 버퍼 (OS 무관)
/// 가림 중 입력된 문자를 모아, 끝부분이 비상 문구와 일치하는지 검사한다.
#[derive(Default)]
pub struct PhraseBuffer {
    buf: String,
}

const MAX_CHARS: usize = 200;
const KEEP_CHARS: usize = 100;

impl PhraseBuffer {
    pub const fn new() -> Self {
        Self { buf: String::new() }
    }

    pub fn push(&mut self, c: char) {
        self.buf.push(c);
        // 무한히 커지지 않도록 뒤쪽만 유지
        if self.buf.chars().count() > MAX_CHARS {
            let skip = self.buf.chars().count() - KEEP_CHARS;
            self.buf = self.buf.chars().skip(skip).collect();
        }
    }

    pub fn backspace(&mut self) {
        self.buf.pop();
    }

    pub fn clear(&mut self) {
        self.buf.clear();
    }

    pub fn len(&self) -> usize {
        self.buf.chars().count()
    }

    /// 빈 문구는 항상 불일치 (비상 해제 비활성)
    pub fn matches(&self, phrase: &str) -> bool {
        !phrase.is_empty() && self.buf.ends_with(phrase)
    }
}
//#endregion

//#region 단위 테스트
#[cfg(test)]
mod tests {
    use super::*;

    fn typed(s: &str) -> PhraseBuffer {
        let mut b = PhraseBuffer::new();
        s.chars().for_each(|c| b.push(c));
        b
    }

    #[test]
    fn matches_suffix() {
        assert!(typed("asdf let me go").matches("let me go"));
        assert!(!typed("let me g").matches("let me go"));
    }

    #[test]
    fn empty_phrase_never_matches() {
        assert!(!typed("anything").matches(""));
    }

    #[test]
    fn backspace_corrects() {
        let mut b = typed("let me gx");
        b.backspace();
        b.push('o');
        assert!(b.matches("let me go"));
    }

    #[test]
    fn bounded_size() {
        let mut b = PhraseBuffer::new();
        for _ in 0..1000 {
            b.push('a');
        }
        assert!(b.len() <= MAX_CHARS);
        "hello".chars().for_each(|c| b.push(c));
        assert!(b.matches("hello"));
    }
}
//#endregion
