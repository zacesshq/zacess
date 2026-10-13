"""Regenerates the quality suite fixtures and their expected output.

Every fixture is written from the content below, which was written for this repository and is
covered by its licence. Nothing is copied from a published source, so every file may be stored in
a public repository. The only outside material is the DejaVu font, whose licence allows embedding.

Usage, from the repository root:

    python3 -m venv .venv && .venv/bin/pip install reportlab pillow
    .venv/bin/python tests/fixtures/generate.py

Output is deterministic: the same script gives byte-identical images, so a regenerated fixture only
shows up in a diff when its content changed.
"""

import io
import json
import random
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter, ImageFont
from reportlab.lib.utils import simpleSplit
from reportlab.pdfbase import pdfmetrics
from reportlab.pdfbase.ttfonts import TTFont
from reportlab.pdfgen import canvas

HERE = Path(__file__).resolve().parent
FONT_DIR = Path("/usr/share/fonts/truetype/dejavu")
REGULAR = FONT_DIR / "DejaVuSans.ttf"
BOLD = FONT_DIR / "DejaVuSans-Bold.ttf"
SERIF = FONT_DIR / "DejaVuSerif.ttf"
SERIF_BOLD = FONT_DIR / "DejaVuSerif-Bold.ttf"

pdfmetrics.registerFont(TTFont("Sans", str(REGULAR)))
pdfmetrics.registerFont(TTFont("Sans-Bold", str(BOLD)))
pdfmetrics.registerFont(TTFont("Serif", str(SERIF)))
pdfmetrics.registerFont(TTFont("Serif-Bold", str(SERIF_BOLD)))


def h(level, text):
    return {"type": "heading", "level": level, "text": text}


def p(text):
    return {"type": "paragraph", "text": text}


def li(text):
    return {"type": "list_item", "text": text}


# --- content -------------------------------------------------------------------------------------

SLIDES = [
    [h(1, "Signals and Systems"), p("Week 3: Fourier series"), p("Lecture slides for EE2001")],
    [
        h(2, "Why Fourier series?"),
        li("Any periodic signal is a sum of sinusoids"),
        li("Each harmonic has its own amplitude and phase"),
        li("A filter acts on each harmonic separately"),
    ],
    [
        h(2, "Worked example"),
        p("A square wave at 50 Hz has only odd harmonics."),
        li("Third harmonic: 150 Hz"),
        li("Fifth harmonic: 250 Hz"),
        li("Seventh harmonic: 350 Hz"),
    ],
    [
        h(2, "Summary"),
        p("Periodic signals can be analysed one harmonic at a time."),
        p("Next week: the Fourier transform for signals that do not repeat."),
    ],
]

TEXTBOOK_TITLE = h(1, "Chapter 4: Operational Amplifiers")
TEXTBOOK_LEFT = [
    h(2, "4.1 The ideal op-amp"),
    p(
        "An operational amplifier is a voltage amplifier with a very high gain. In the ideal model "
        "the input current is zero and the gain is infinite, which makes circuit analysis simple."
    ),
    p(
        "Two rules follow from the ideal model. No current flows into either input. With "
        "negative feedback the two inputs sit at the same voltage."
    ),
    h(2, "4.2 The inverting amplifier"),
    p(
        "The input signal reaches the inverting input through a resistor. A second resistor feeds "
        "the output back to the same input, which sets the gain of the stage."
    ),
]
TEXTBOOK_RIGHT = [
    p(
        "Because the inverting input is held at zero volts, the current through the input resistor "
        "equals the current through the feedback resistor. The output is therefore the input "
        "scaled by the ratio of the two resistors, with the sign reversed."
    ),
    h(2, "4.3 The non-inverting amplifier"),
    p(
        "Here the signal drives the non-inverting input directly. The gain is one plus the ratio "
        "of the resistors, so this stage can never attenuate the signal."
    ),
    p(
        "Its input impedance is very high, which makes it a good buffer between a sensor and the "
        "rest of a circuit."
    ),
]

NUMBERS = [
    h(1, "Potential divider notes"),
    p("Supply: Vin = 12 V, tolerance ± 5%"),
    li("R1 = 4.7 kΩ"),
    li("R2 = 10 kΩ"),
    li("Vout = Vin × R2 / (R1 + R2)"),
    li("Vout = 12 × 10 / 14.7 = 8.16 V"),
    p("A load current of 2.2 mA through 160 Ω shifts the output by −0.35 V."),
    p("Phase shift at 1.5 kHz: 45° (π/4 rad)."),
]

HANDOUT = [
    h(1, "Laboratory safety handout"),
    p("Read this sheet before your first practical session."),
    h(2, "Before you start"),
    li("Check that every lead is undamaged."),
    li("Keep liquids away from the bench."),
    li("Switch off the supply before changing a circuit."),
    h(2, "If something goes wrong"),
    p("Press the red isolation button and tell a demonstrator at once."),
]


def table(rows):
    return {"type": "table", "text": "\n".join("\t".join(row) for row in rows), "rows": rows}


def eq(text):
    return {"type": "equation", "text": text}


TECHNICAL_TABLE = [
    ["Component", "Value", "Tolerance"],
    ["R1", "4.7 kΩ", "± 5%"],
    ["R2", "10 kΩ", "± 1%"],
    ["C1", "100 nF", "± 10%"],
]

TECHNICAL = [
    h(1, "Potential divider design"),
    p("Values measured on the bench after soldering."),
    table(TECHNICAL_TABLE),
    p("The output follows from the divider rule and the power from Ohm's law."),
    eq("Vout = Vin × R2 / (R1 + R2)"),
    eq("P = I2 R"),
]

# --- PDF rendering -------------------------------------------------------------------------------


def wrap(text, font, size, width):
    return simpleSplit(text, font, size, width)


def draw_lines(c, lines, font, size, x, y, leading):
    c.setFont(font, size)
    for line in lines:
        c.drawString(x, y, line)
        y -= leading
    return y


def write_slides(path):
    width, height = 960, 540
    c = canvas.Canvas(str(path), pagesize=(width, height), invariant=1)
    c.setTitle("Signals and Systems week 3")
    for slide in SLIDES:
        y = height - 90
        for block in slide:
            if block["type"] == "heading":
                size = 44 if block["level"] == 1 else 36
                y = draw_lines(c, [block["text"]], "Sans-Bold", size, 60, y, size * 1.4) - 10
            elif block["type"] == "list_item":
                c.setFont("Sans", 26)
                c.drawString(70, y, "•")
                y = draw_lines(c, wrap(block["text"], "Sans", 26, 780), "Sans", 26, 105, y, 36) - 12
            else:
                y = draw_lines(c, wrap(block["text"], "Sans", 26, 820), "Sans", 26, 60, y, 36) - 12
        c.showPage()
    c.save()


def column_blocks(c, blocks, x, y, width):
    for block in blocks:
        if block["type"] == "heading":
            y -= 6
            y = draw_lines(c, wrap(block["text"], "Serif-Bold", 13, width), "Serif-Bold", 13, x, y, 17) - 4
        else:
            y = draw_lines(c, wrap(block["text"], "Serif", 10.5, width), "Serif", 10.5, x, y, 14) - 8
    return y


def write_textbook(path):
    width, height = 595, 842
    c = canvas.Canvas(str(path), pagesize=(width, height), invariant=1)
    c.setTitle("Chapter 4")
    # the right column is drawn first so raw draw order disagrees with reading order
    column_blocks(c, TEXTBOOK_RIGHT, 315, height - 130, 230)
    draw_lines(c, [TEXTBOOK_TITLE["text"]], "Serif-Bold", 20, 50, height - 80, 26)
    column_blocks(c, TEXTBOOK_LEFT, 50, height - 130, 230)
    c.setFont("Serif", 9)
    c.drawString(width / 2 - 6, 40, "47")
    c.showPage()
    c.save()


def write_flow_pdf(path, blocks, title):
    width, height = 595, 842
    c = canvas.Canvas(str(path), pagesize=(width, height), invariant=1)
    c.setTitle(title)
    y = height - 80
    for block in blocks:
        if block["type"] == "heading":
            size = 20 if block["level"] == 1 else 15
            y = draw_lines(c, [block["text"]], "Sans-Bold", size, 60, y, size * 1.4) - 6
        elif block["type"] == "list_item":
            c.setFont("Sans", 12)
            c.drawString(70, y, "•")
            y = draw_lines(c, wrap(block["text"], "Sans", 12, 440), "Sans", 12, 88, y, 17) - 5
        else:
            y = draw_lines(c, wrap(block["text"], "Sans", 12, 470), "Sans", 12, 60, y, 17) - 10
    c.showPage()
    c.save()


def write_technical(path):
    """A technical page: an aligned table, then equations with real sub- and superscripts."""
    width, height = 595, 842
    c = canvas.Canvas(str(path), pagesize=(width, height), invariant=1)
    c.setTitle("Potential divider design")
    y = height - 80
    y = draw_lines(c, [TECHNICAL[0]["text"]], "Sans-Bold", 20, 60, y, 28) - 6
    y = draw_lines(c, [TECHNICAL[1]["text"]], "Sans", 12, 60, y, 17) - 12
    columns = [60, 220, 340]
    for r, row in enumerate(TECHNICAL_TABLE):
        font = "Sans-Bold" if r == 0 else "Sans"
        c.setFont(font, 12)
        for x, cell in zip(columns, row):
            c.drawString(x, y, cell)
        y -= 20
    y -= 10
    y = draw_lines(c, wrap(TECHNICAL[3]["text"], "Sans", 12, 470), "Sans", 12, 60, y, 17) - 12

    def scripted(parts):
        # parts: (text, rise) with rise -3 for a subscript and 5 for a superscript
        nonlocal y
        t = c.beginText(80, y)
        for text, rise in parts:
            t.setFont("Sans", 9 if rise else 13)
            t.setRise(rise)
            t.textOut(text)
        c.drawText(t)
        y -= 26

    scripted([("V", 0), ("out", -3), (" = V", 0), ("in", -3), (" × R", 0), ("2", -3), (" / (R", 0), ("1", -3), (" + R", 0), ("2", -3), (")", 0)])
    scripted([("P = I", 0), ("2", 5), (" R", 0)])
    c.showPage()
    c.save()


# --- image rendering, standing in for a scan or a photo ---------------------------------------------


def render_image(blocks, dpi=200, rotate=0.4, seed=7):
    scale = dpi / 72
    width, height = int(595 * scale), int(842 * scale)
    img = Image.new("L", (width, height), 246)
    draw = ImageDraw.Draw(img)
    fonts = {}

    def font(path, size):
        key = (path, size)
        if key not in fonts:
            fonts[key] = ImageFont.truetype(str(path), int(size * scale))
        return fonts[key]

    def text_lines(text, f, max_width):
        words, lines, line = text.split(), [], ""
        for word in words:
            trial = f"{line} {word}".strip()
            if f.getlength(trial) <= max_width:
                line = trial
            else:
                lines.append(line)
                line = word
        lines.append(line)
        return lines

    y = 70 * scale
    left = 60 * scale
    for block in blocks:
        if block["type"] == "heading":
            size = 20 if block["level"] == 1 else 15
            f = font(BOLD, size)
            draw.text((left, y), block["text"], font=f, fill=25)
            y += size * 1.6 * scale
        else:
            f = font(REGULAR, 12)
            x = left
            if block["type"] == "list_item":
                draw.text((left + 10 * scale, y), "•", font=f, fill=25)
                x = left + 28 * scale
            for line in text_lines(block["text"], f, (470 * scale) - (x - left)):
                draw.text((x, y), line, font=f, fill=25)
                y += 17 * scale
            y += 8 * scale

    # a slight tilt, soft focus and paper speckle, like a real scan
    img = img.rotate(rotate, resample=Image.BICUBIC, fillcolor=246)
    img = img.filter(ImageFilter.GaussianBlur(0.6))
    rng = random.Random(seed)
    px = img.load()
    for _ in range(width * height // 400):
        x, y = rng.randrange(width), rng.randrange(height)
        px[x, y] = rng.choice((150, 190, 255))
    return img


def write_png(path, img):
    img.save(path, optimize=True)


def write_image_pdf(path, img, title):
    buffer = io.BytesIO()
    img.save(buffer, format="PNG", optimize=True)
    buffer.seek(0)
    c = canvas.Canvas(str(path), pagesize=(595, 842), invariant=1)
    c.setTitle(title)
    from reportlab.lib.utils import ImageReader

    c.drawImage(ImageReader(buffer), 0, 0, width=595, height=842)
    c.showPage()
    c.save()


# --- whiteboard captures ---------------------------------------------------------------------------
BOARD_SIZE = (1200, 800)
BOARD_FIRST = [
    (80, 70, "Week 3  Fourier series"),
    (80, 190, "f(t) = a0/2 + sum an cos(n w0 t) + bn sin(n w0 t)"),
    (80, 310, "T = 1/f0   and   w0 = 2 pi f0"),
]
BOARD_ADDED = [(80, 430, "an = (2/T) integral f(t) cos(n w0 t) dt")]
BOARD_REWRITTEN = [(80, 310, "square wave: only odd n, bn = 4/(n pi)")]
BOARD_NEXT = [
    (80, 70, "Week 4  Laplace transform"),
    (80, 190, "F(s) = integral from 0 to inf of f(t) e^(-st) dt"),
    (80, 310, "step: 1/s    ramp: 1/s^2    e^(-at): 1/(s+a)"),
]
NOTES_PAGE = [
    (130, 78, "Fourier series notes"),
    (130, 138, "periodic signal = sum of sines"),
    (130, 198, "fundamental f0 = 1/T"),
    (130, 258, "harmonics at n f0"),
    (130, 318, "square wave: odd harmonics only"),
    (130, 378, "amplitude falls as 1/n"),
]


def board_background(seed, light=(250, 245, 236), tilt=1.0):
    """An off-white board lit from one side, darkening by 18 levels across its width (the sign of
    the tilt says which side) and 10 levels down its height, with faint noise."""
    rnd = random.Random(seed)
    width, height = BOARD_SIZE
    img = Image.new("RGB", (width, height), light)
    px = img.load()
    for y in range(height):
        for x in range(width):
            shade = int(18 * (x / width) * tilt + 10 * (y / height))
            r, g, b = light
            n = rnd.randint(-3, 3)
            px[x, y] = (max(0, r - shade + n), max(0, g - shade + n), max(0, b - shade + n))
    return img


def marker_text(img, lines, offset=(0, 0), colour=(24, 32, 96), size=44, seed=1):
    """Marker strokes: text with a slight per-line wobble, softened like a photo."""
    rnd = random.Random(seed)
    layer = Image.new("RGBA", img.size, (0, 0, 0, 0))
    d = ImageDraw.Draw(layer)
    font = ImageFont.truetype(str(REGULAR), size)
    for x, y, text in lines:
        dx = offset[0] + rnd.randint(-2, 2)
        dy = offset[1] + rnd.randint(-2, 2)
        d.text((x + dx, y + dy), text, font=font, fill=colour + (255,))
    layer = layer.rotate(rnd.uniform(-0.4, 0.4), resample=Image.BICUBIC)
    layer = layer.filter(ImageFilter.GaussianBlur(0.8))
    img.paste(layer, (0, 0), layer)
    return img


def write_whiteboard(out):
    """A board photographed through a lecture, then a page of notes, with a sidecar of what each is."""
    out.mkdir(exist_ok=True)
    captures = []

    def save(img, name, board, expect, lines):
        if name.endswith(".jpg"):
            img.save(out / name, quality=85)
        else:
            img.save(out / name, optimize=True)
        captures.append({"file": name, "board": board, "expect": expect, "text": [t for _, _, t in lines]})

    # the first state of the board
    save(marker_text(board_background(1), BOARD_FIRST), "board-1-first.jpg", "fourier", "new", BOARD_FIRST)
    # the same board a moment later: different light, a small shift, more noise
    relit = marker_text(board_background(2, light=(236, 232, 226), tilt=-1.0), BOARD_FIRST, offset=(6, -4))
    save(relit.filter(ImageFilter.GaussianBlur(0.4)), "board-2-relit.jpg", "fourier", "repeat", BOARD_FIRST)
    # a line added under the others
    added = BOARD_FIRST + BOARD_ADDED
    save(marker_text(board_background(3), added), "board-3-added.jpg", "fourier", "changed", added)
    # the third line rubbed out and rewritten, the rest kept
    rewritten = BOARD_FIRST[:2] + BOARD_REWRITTEN + BOARD_ADDED
    save(marker_text(board_background(4), rewritten), "board-4-rewritten.jpg", "fourier", "changed", rewritten)
    # a wiped board with the next topic
    save(marker_text(board_background(5), BOARD_NEXT), "board-5-new-topic.jpg", "laplace", "changed", BOARD_NEXT)

    # a page of handwritten notes on lined paper, neat print in pen
    page = Image.new("RGB", (900, 1200), (252, 251, 246))
    d = ImageDraw.Draw(page)
    for y in range(120, 1200, 60):
        d.line([(40, y), (860, y)], fill=(190, 205, 225), width=2)
    d.line([(110, 0), (110, 1200)], fill=(230, 160, 160), width=2)
    page = marker_text(page, NOTES_PAGE, colour=(20, 30, 110), size=36, seed=9)
    save(page, "notes-1.png", "notes", "new", NOTES_PAGE)

    sidecar = {
        "source": (
            "Synthetic captures drawn with Pillow from text written for this repository: an off-white "
            "board with an illumination gradient and noise, marker text in the DejaVu Sans font with a "
            "slight wobble and blur, plus lined paper for the notes page. Nothing was photographed. The "
            "boards are saved as JPEG at quality 85, as a phone camera would save them. The notes "
            "page is saved as PNG."
        ),
        "captures": captures,
    }
    (out / "captures.json").write_text(json.dumps(sidecar, indent=2) + "\n", encoding="utf-8")


# --- lecture slides photographed from a projector -------------------------------------------------
SLIDE_SIZE = (960, 540)
CAMERA_SIZE = (1040, 620)
SLIDE_ONE = ("Fourier series", [
    "Any periodic signal is a sum of sines",
    "Harmonics sit at multiples of f0",
    "A square wave has odd harmonics only",
], "light")
SLIDE_TWO = ("Laplace transform", [
    "F(s) is the integral of f(t) e^(-st) dt",
    "Poles give the natural response",
    "Zeros shape the frequency response",
], "light")


def draw_slide(title, bullets, theme, shown=None):
    """A slide as the projector shows it. `shown` limits how many bullets are on it (a build)."""
    light = theme == "light"
    background = (250, 250, 247) if light else (18, 32, 64)
    ink = (20, 24, 40) if light else (240, 244, 250)
    accent = (30, 80, 160) if light else (120, 190, 255)
    img = Image.new("RGB", SLIDE_SIZE, background)
    d = ImageDraw.Draw(img)
    d.rectangle([0, 0, SLIDE_SIZE[0], 96], fill=accent)
    d.text((40, 20), title, font=ImageFont.truetype(str(BOLD), 48), fill=(255, 255, 255))
    body = ImageFont.truetype(str(REGULAR), 34)
    for i, text in enumerate(bullets[: len(bullets) if shown is None else shown]):
        y = 150 + i * 90
        d.ellipse([46, y + 12, 62, y + 28], fill=accent)
        d.text((86, y), text, font=body, fill=ink)
    return img


def draw_diagram():
    """A different kind of slide: boxes and arrows on a dark background."""
    img = draw_slide("Block diagram", [], "dark")
    d = ImageDraw.Draw(img)
    font = ImageFont.truetype(str(REGULAR), 30)
    for i, label in enumerate(["input", "filter", "output"]):
        x = 70 + i * 300
        d.rectangle([x, 230, x + 210, 330], outline=(240, 244, 250), width=4)
        d.text((x + 40, 262), label, font=font, fill=(240, 244, 250))
        if i < 2:
            d.line([x + 210, 280, x + 300, 280], fill=(120, 190, 255), width=5)
            d.polygon([(x + 300, 280), (x + 280, 268), (x + 280, 292)], fill=(120, 190, 255))
    points = [(70 + k * 8, 470 - int(70 * abs(((k * 8) % 200) - 100) / 100)) for k in range(100)]
    d.line(points, fill=(120, 190, 255), width=4)
    return img


def photograph(slide, seed, shift=(0, 0), turn=0.0, light=1.0, gradient=0.0, blur=0.0):
    """A phone photo of a projected slide: wall around it, vignette, noise, shake and exposure."""
    rnd = random.Random(seed)
    width, height = CAMERA_SIZE
    wall = Image.new("RGB", CAMERA_SIZE, (58, 56, 52))
    wall.paste(slide, (40 + shift[0], 40 + shift[1]))
    if turn:
        wall = wall.rotate(turn, resample=Image.BICUBIC, fillcolor=(58, 56, 52))
    if blur:
        wall = wall.filter(ImageFilter.GaussianBlur(blur))
    px = wall.load()
    for y in range(height):
        for x in range(width):
            fall = 1.0 - 0.25 * (((x - width / 2) / width) ** 2 + ((y - height / 2) / height) ** 2) * 2
            tilt = 1.0 + gradient * (x / width - 0.5)
            noise = rnd.randint(-3, 3)
            r, g, b = px[x, y]
            k = light * fall * tilt
            px[x, y] = (
                max(0, min(255, int(r * k) + noise)),
                max(0, min(255, int(g * k) + noise)),
                max(0, min(255, int(b * k * 0.97) + noise)),
            )
    return wall


def write_slide_frames(out):
    """Frames from a lecture camera, in the order they arrive, with a sidecar of what each is."""
    out.mkdir(exist_ok=True)
    frames = []

    def save(name, img, expect, note):
        img.save(out / name, quality=88)
        frames.append({"file": name, "expect": expect, "note": note})

    one = draw_slide(*SLIDE_ONE)
    save("a1-slide-one.jpg", photograph(one, 1), "new_slide", "slide one, first sight")
    save("a2-slide-one-again.jpg", photograph(one, 1), "duplicate", "the same photograph again, pixel for pixel")
    save("a3-slide-one-shake.jpg", photograph(one, 2, shift=(7, -5), turn=0.7, blur=0.6), "duplicate", "camera shake")
    save("a4-slide-one-dim.jpg", photograph(one, 3, light=0.62, gradient=0.35), "duplicate", "lighting change")
    save("a5-slide-one-soft.jpg", photograph(one, 4, blur=2.2), "duplicate", "out of focus")

    steps = [draw_slide(*SLIDE_TWO, shown=n) for n in (1, 2, 3)]
    save("b1-slide-two-step1.jpg", photograph(steps[0], 5), "new_slide", "slide two with its first bullet")
    save("b2-slide-two-step1-again.jpg", photograph(steps[0], 6, shift=(3, 2)), "duplicate", "same step, a moment later")
    save("b3-slide-two-step2.jpg", photograph(steps[1], 7), "build", "second bullet appears")
    save("b4-slide-two-step2-again.jpg", photograph(steps[1], 8, shift=(-3, 2), light=0.9), "duplicate", "same step")
    save("b5-slide-two-step3.jpg", photograph(steps[2], 9), "build", "third bullet appears")
    save("b6-slide-two-step3-again.jpg", photograph(steps[2], 10, shift=(2, -3)), "duplicate", "same step")

    diagram = draw_diagram()
    blend = Image.blend(steps[2], diagram, 0.5)
    save("c1-transition.jpg", photograph(blend, 11), "pending", "half way through a fade between slides")
    save("c2-diagram.jpg", photograph(diagram, 12), "new_slide", "slide three, a diagram")
    save("c3-diagram-again.jpg", photograph(diagram, 13, shift=(-4, 3)), "duplicate", "same slide")
    save("d1-nothing.jpg", Image.new("RGB", CAMERA_SIZE, (6, 6, 6)), "unusable", "the lens covered")
    save("d2-back-to-one.jpg", photograph(one, 14, light=0.85, shift=(2, 2)), "revisit", "the lecturer goes back")
    save("d3-back-to-two-step1.jpg", photograph(steps[0], 15, light=0.9, shift=(-2, 1)), "revisit", "back to slide two, before its bullets are built")

    sidecar = {
        "source": (
            "Synthetic frames drawn with Pillow from text written for this repository: three slides "
            "(two light, one dark with a diagram) photographed with a simulated phone camera that adds "
            "a wall around the projection, vignetting, noise, shake, exposure changes and blur. Nothing "
            "was photographed. Slide two is built in three steps."
        ),
        "frames": frames,
    }
    (out / "frames.json").write_text(json.dumps(sidecar, indent=2) + "\n", encoding="utf-8")


# --- manifest -------------------------------------------------------------------------------------


def expected(name, category, file, text_layer, pages):
    blocks = []
    for number, page_blocks in enumerate(pages, start=1):
        for block in page_blocks:
            blocks.append({"page": number, **{k: v for k, v in block.items() if k != "rows"}})
    return {
        "name": name,
        "category": category,
        "file": file,
        "has_text_layer": text_layer,
        "pages": len(pages),
        "blocks": blocks,
    }


def main():
    fixtures = []

    write_slides(HERE / "slides-signals.pdf")
    fixtures.append(expected("slides-signals", "slides", "slides-signals.pdf", True, SLIDES))

    write_textbook(HERE / "textbook-two-column.pdf")
    fixtures.append(
        expected(
            "textbook-two-column",
            "textbook",
            "textbook-two-column.pdf",
            True,
            [[TEXTBOOK_TITLE, *TEXTBOOK_LEFT, *TEXTBOOK_RIGHT]],
        )
    )

    write_flow_pdf(HERE / "notes-numbers.pdf", NUMBERS, "Potential divider notes")
    fixtures.append(expected("notes-numbers", "numbers", "notes-numbers.pdf", True, [NUMBERS]))

    write_technical(HERE / "notes-technical.pdf")
    fixtures.append(expected("notes-technical", "numbers", "notes-technical.pdf", True, [TECHNICAL]))

    numbers_img = render_image(NUMBERS, rotate=-0.3, seed=11)
    write_png(HERE / "notes-numbers-scan.png", numbers_img)
    fixtures.append(
        expected("notes-numbers-scan", "numbers", "notes-numbers-scan.png", False, [NUMBERS])
    )

    handout_img = render_image(HANDOUT)
    write_png(HERE / "handout-scan.png", handout_img)
    fixtures.append(expected("handout-scan", "scanned", "handout-scan.png", False, [HANDOUT]))

    write_image_pdf(HERE / "handout-scan.pdf", handout_img, "Laboratory safety handout")
    fixtures.append(expected("handout-scan-pdf", "scanned", "handout-scan.pdf", False, [HANDOUT]))

    write_whiteboard(HERE / "whiteboard")
    write_slide_frames(HERE / "slides")

    for fixture in fixtures:
        out = HERE / f"{fixture['name']}.expected.json"
        out.write_text(json.dumps(fixture, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    manifest = [{"name": f["name"], "category": f["category"], "file": f["file"]} for f in fixtures]
    (HERE / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
