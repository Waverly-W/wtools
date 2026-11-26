/** 字数统计 */
export interface TextStats {
  chars: number;
  charsNoSpace: number;
  han: number;
  words: number;
  numbers: number;
  punctuation: number;
  spaces: number;
  lines: number;
  paragraphs: number;
}

export function textStats(text: string): TextStats {
  const count = (re: RegExp) => text.match(re)?.length ?? 0;
  const chars = [...text].length;
  const spaces = count(/\s/gu);
  return {
    chars,
    charsNoSpace: chars - spaces,
    han: count(/\p{Script=Han}/gu),
    words: count(/[A-Za-z]+(?:['’-][A-Za-z]+)*/g),
    numbers: count(/\d+(?:[.,]\d+)*/g),
    punctuation: count(/[\p{P}\p{S}]/gu),
    spaces,
    lines: text ? text.split(/\r\n|\r|\n/).length : 0,
    paragraphs: text.split(/\n\s*\n/).filter((p) => p.trim()).length,
  };
}

export const STAT_LABELS: [keyof TextStats, string][] = [
  ["chars", "字符（含空白）"],
  ["charsNoSpace", "字符（不含空白）"],
  ["han", "汉字"],
  ["words", "英文单词"],
  ["numbers", "数字"],
  ["punctuation", "标点符号"],
  ["spaces", "空白字符"],
  ["lines", "行数"],
  ["paragraphs", "段落"],
];
