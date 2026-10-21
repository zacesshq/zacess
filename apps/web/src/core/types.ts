// The JSON forms the core reads and writes. Only the fields the app looks at are typed; the rest
// pass through untouched.

export type ContrastTheme =
  | "dark_on_light"
  | "light_on_dark"
  | "yellow_on_black"
  | "black_on_cream"
  | "high_contrast_dark"
  | "high_contrast_light"
  | "deep_contrast_dark"
  | "deep_contrast_light";
export type PresetId = "low_vision" | "dyslexia_friendly" | "reduced_visual_clutter" | "screen_reader";

export interface ReadingProfile {
  schema_version: number;
  name: string;
  font: { family: string; fallback: "sans_serif" | "serif" | "monospace" };
  font_size_pt: number;
  line_spacing: number;
  paragraph_spacing: number;
  character_spacing: number;
  column_width: number;
  contrast: ContrastTheme;
  cursor_size: "standard" | "large" | "extra_large";
  reading_ruler: { enabled: boolean; position: number; lines: number };
  single_column?: boolean;
  reduce_motion?: boolean;
  hide_decoration?: boolean;
}

export interface Preset {
  id: PresetId;
  name: string;
  profile: ReadingProfile;
}

export interface Document {
  schema_version: number;
  title: string;
  source: { file_name: string; format: string };
  pages: { number: number; width: number; height: number; unit: string }[];
  blocks: unknown[];
  [key: string]: unknown;
}

export interface Summary {
  title: string;
  blocks: number;
  pages: number;
  uncertain: number;
  images_without_alt: number;
}

export interface TextItem {
  x: number;
  baseline: number;
  width: number;
  size: number;
  bold: boolean;
  text: string;
}

export interface OcrWordInput {
  text: string;
  left: number;
  top: number;
  width: number;
  height: number;
  /** From 0 to 100, as OCR engines report it. */
  confidence: number;
}

export interface OcrLineInput {
  left: number;
  top: number;
  width: number;
  height: number;
  words: OcrWordInput[];
}

export interface OcrInput {
  width: number;
  height: number;
  paragraphs: { lines: OcrLineInput[] }[];
}

/** A link area on a PDF page, in the page's units from its top-left corner. */
export interface LinkInput {
  url: string;
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface PageInput {
  number: number;
  width: number;
  height: number;
  items?: TextItem[];
  ocr?: OcrInput;
  links?: LinkInput[];
}

export interface PagesInput {
  file_name: string;
  title?: string;
  format: "pdf" | "image";
  pages: PageInput[];
}
