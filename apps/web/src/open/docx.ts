// Word files become HTML with mammoth, which keeps headings, lists, tables and image descriptions,
// and that HTML goes through the core's own HTML reader.
export async function docxToHtml(data: ArrayBuffer): Promise<{ html: string; warnings: string[] }> {
  const mammoth = await import("mammoth");
  const convert = (mammoth.default ?? mammoth).convertToHtml;
  // the browser build reads arrayBuffer and the Node build, which the tests use, reads buffer
  const input = { arrayBuffer: data, buffer: data } as unknown as Parameters<typeof convert>[0];
  const result = await convert(input);
  return {
    html: result.value,
    warnings: result.messages.filter((m) => m.type === "warning").map((m) => m.message),
  };
}
