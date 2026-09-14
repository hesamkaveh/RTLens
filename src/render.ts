/**
 * Build the HUD's DOM from an analysed document.
 *
 * Deliberately mirrors `rtlens_core::render` element for element and class for class, so
 * the offline harness the engine is tested against and the shipping window are the same
 * layout. Any change here needs the matching change there.
 */

export type Dir = "ltr" | "rtl";
export type Segment = { kind: "text" | "code"; text: string };

export type Cell = { dir: Dir; align: Dir; segments: Segment[] };

/**
 * Table membership. Table-wide facts repeat on every row so the wire format stays a flat
 * list of lines; consecutive rows sharing an `id` are one table.
 */
export type TableRow = {
  id: number;
  columns: number;
  dir: Dir;
  head: boolean;
  framed: boolean;
  cells: Cell[];
};

export type Line = {
  lead: string;
  tail: string;
  boxed: boolean;
  code: boolean;
  dir: Dir;
  segments: Segment[];
  table?: TableRow;
};

export type CaptureSource = "selection" | "clipboardFallback" | "empty";

export type Doc = {
  lines: Line[];
  source: CaptureSource;
  diffMode: boolean;
};

function isBlank(line: Line): boolean {
  return (
    line.lead.trim() === "" &&
    line.tail.trim() === "" &&
    line.segments.every((s) => s.text.trim() === "")
  );
}

/** Keep rendered text selectable inside the HUD's deep window-drag region. */
function preserveTextSelection(element: HTMLElement): void {
  element.setAttribute("data-tauri-drag-region", "false");
}

function gutter(text: string, tail = false): HTMLElement {
  const span = document.createElement("span");
  span.className = tail ? "rtl-gutter rtl-gutter--tail" : "rtl-gutter";
  span.textContent = text;
  preserveTextSelection(span);
  return span;
}

function appendSegments(target: HTMLElement, segments: Segment[]): void {
  for (const segment of segments) {
    if (segment.kind === "code") {
      const code = document.createElement("span");
      code.className = "rtl-code";
      code.setAttribute("dir", "ltr");
      code.textContent = segment.text;
      target.append(code);
    } else {
      target.append(document.createTextNode(segment.text));
    }
  }
}

/**
 * A recovered table, as one grid. Every row of a table has to live in the same grid or the
 * browser has no way to give them shared column widths.
 */
function tableElement(rows: TableRow[]): HTMLElement {
  const table = document.createElement("div");
  table.className = rows[0].framed ? "rtl-table rtl-table--framed" : "rtl-table";
  // The table's direction is its column order: a Persian table starts at the right.
  table.setAttribute("dir", rows[0].dir);
  table.style.setProperty("--rtl-cols", String(rows[0].columns));
  for (const row of rows) {
    const rowEl = document.createElement("div");
    rowEl.className = row.head ? "rtl-row rtl-row--head" : "rtl-row";
    for (const cell of row.cells) {
      const cellEl = document.createElement("div");
      // `dir` is the cell's own direction; the class is its column's, which is what the
      // text aligns to so a column reads as one column.
      cellEl.className = `rtl-cell rtl-cell--${cell.align}`;
      cellEl.setAttribute("dir", cell.dir);
      preserveTextSelection(cellEl);
      appendSegments(cellEl, cell.segments);
      rowEl.append(cellEl);
    }
    table.append(rowEl);
  }
  return table;
}

function lineElement(line: Line): HTMLElement {
  const el = document.createElement("div");
  el.classList.add("rtl-line");
  if (line.code) el.classList.add("rtl-line--code");
  if (isBlank(line)) el.classList.add("rtl-line--blank");
  // A box frame is structural and stays physical; every unboxed RTL line belongs on the
  // reading-start side and so flips with the line. The presence of a lead is unrelated
  // to the line's writing direction.
  if (!line.boxed && line.dir === "rtl") {
    el.classList.add("rtl-line--rtl");
  }

  if (line.lead !== "") el.append(gutter(line.lead));

  const content = document.createElement("span");
  content.className = "rtl-content";
  content.setAttribute("dir", line.dir);
  preserveTextSelection(content);
  appendSegments(content, line.segments);
  el.append(content);

  if (line.tail !== "") el.append(gutter(line.tail, true));
  return el;
}

export function renderDoc(target: HTMLElement, doc: Doc): void {
  const fragment = document.createDocumentFragment();
  let i = 0;
  while (i < doc.lines.length) {
    const first = doc.lines[i].table;
    if (first) {
      const rows: TableRow[] = [first];
      let end = i + 1;
      for (; end < doc.lines.length; end += 1) {
        const next = doc.lines[end].table;
        if (!next || next.id !== first.id) break;
        rows.push(next);
      }
      fragment.append(tableElement(rows));
      i = end;
    } else {
      fragment.append(lineElement(doc.lines[i]));
      i += 1;
    }
  }
  target.replaceChildren(fragment);
}
