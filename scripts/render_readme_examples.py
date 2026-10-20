# Copyright (C) 2026 Toit contributors.
"""Render documentation figures from the checked-in evidence excerpts.

Requires Python Playwright and an installed Chromium/Chrome browser.
Run from the repository root: python3 scripts/render_readme_examples.py
These figures present saved measurements; they do not run the solver.
"""

import argparse
import base64
import hashlib
import html
import json
from pathlib import Path
import shutil
import select
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parents[1]
IMAGES = ROOT / "docs/images"
SAMPLES = IMAGES / "examples"
CSS = """
* { box-sizing: border-box }
body { margin: 0; color: #182b40; font: 20px/1.45 Arial, sans-serif; background: #edf2f6 }
main { width: 1280px; padding: 38px 42px 28px }
.eyebrow { color: #526a80; font-size: 15px; font-weight: bold; letter-spacing: 2px; text-transform: uppercase }
h1 { font-size: 35px; letter-spacing: -.8px; margin: 8px 0 8px }
.intro { color: #526477; margin: 0 0 26px; font-size: 19px }
.grid { display: grid; grid-template-columns: 1fr 1fr; gap: 22px }
.card { background: white; border: 1px solid #d8e1e8; border-radius: 16px; overflow: hidden }
.inner { padding: 24px }
.tag { display: inline-block; background: #e7f4ed; color: #116044; border-radius: 6px; padding: 5px 10px; font-size: 15px; font-weight: bold }
.wrong { color: #8d4b10; background: #fff0dc }
h2 { margin: 15px 0 9px; font-size: 26px; letter-spacing: -.4px }
.crop { height: 114px; background: #f8fafb; border-block: 1px solid #e0e7ed; display: flex; align-items: center; justify-content: center; padding: 18px 14px }
.crop img { max-width: 100%; max-height: 100%; object-fit: contain }
.context { height: 75px; font-size: 18px; color: #516376; margin: 0 0 15px }
.heading, .row { display: flex; align-items: center; justify-content: space-between }
.heading { color: #617488; font-size: 15px; margin-bottom: 10px }
.row { padding: 11px 12px; border-radius: 6px; font-size: 21px; margin-bottom: 5px; background: #f4f6f9 }
.row.good { background: #e7f4ed; color: #116044 }
.row.bad { background: #fff0dc; color: #8d4b10 }
.score { font-variant-numeric: tabular-nums; font-family: monospace; font-size: 20px }
.note { font-size: 16px; color: #526477; margin: 17px 0 0 }
.footer { font-size: 16px; color: #526477; margin-top: 22px }
.mosaic .crop { height: 155px }
.mosaic .crop img { width: 510px; max-height: none; image-rendering: pixelated }
.answer { font-size: 38px; font-weight: bold; color: #116044; margin: 18px 0 4px }
.label { color: #617488; font-size: 15px; text-transform: uppercase; letter-spacing: 1px }
"""


def esc(value):
    return html.escape(str(value))


def image(sample):
    path = SAMPLES / sample["file"]
    content = path.read_bytes()
    assert hashlib.sha256(content).hexdigest() == sample["sha256"], path
    return '<img alt="Observed input pixels" src="data:image/png;base64,' + base64.b64encode(content).decode() + '">'


def opaque_figure(evidence):
    cards = []
    for case in evidence["opaque"]:
        correct = case["top"][0]["name"] == case["truth"]
        rows = []
        for i, item in enumerate(case["top"], 1):
            state = "good" if item["name"] == case["truth"] else ("bad" if i == 1 else "")
            rows.append(f'<div class="row {state}"><span>{i}. {esc(item["name"])}</span>'
                        f'<span class="score">{item["combined_score"]:.2f}</span></div>')
        cards.append(f"""
        <section class="card">
          <div class="inner"><span class="tag {'' if correct else 'wrong'}">
            {'CORRECT FIRST CHOICE' if correct else 'WRONG FIRST CHOICE'}</span>
            <h2>Known answer: “{esc(case['truth'])}”</h2></div>
          <div class="crop">{image(case['image'])}</div>
          <div class="inner">
            <p class="context">{esc(case['context'])}</p>
            <div class="heading"><span>Ranked candidate</span><span>Combined score ↑</span></div>
            {''.join(rows)}
            <p class="note">{case['compatible']} of {case['tested']} supplied candidates remain physically compatible.</p>
          </div>
        </section>""")
    return f"""
    <div class="eyebrow">Opaque masks · Congressional Record, 1964 · PDF page 43</div>
    <h1>Rank candidates using geometry and visible context</h1>
    <p class="intro">Actual scan crops and saved model outputs. Both black bars were added for the experiment.</p>
    <div class="grid">{''.join(cards)}</div>
    <div class="footer">Two examples from a four-case check: 2/4 correct overall. Higher combined score ranks first; scores are not probabilities.</div>
    """


def mosaic_figure(evidence):
    case = evidence["mosaic"]
    rows = "".join(
        f'<div class="row {"good" if i == 1 else ""}"><span>{i}. {esc(item["name"])}</span>'
        f'<span class="score">{item["evidence_score"]:.3f}</span></div>'
        for i, item in enumerate(case["top"], 1)
    )
    return f"""
    <div class="eyebrow">Pixelation · controlled synthetic demonstration</div>
    <h1>Compare candidate renderings with retained pixels</h1>
    <p class="intro">A fresh solver run on a synthetic name, using the supplied 5-pixel grid and 400 candidates.</p>
    <div class="grid mosaic">
      <section class="card">
        <div class="inner"><span class="label">Observed input · enlarged with nearest-neighbor scaling</span></div>
        <div class="crop">{image(case['image'])}</div>
        <div class="inner"><span class="label">First-ranked candidate / known fixture answer</span>
          <div class="answer">{esc(case['truth'])}</div>
          <div class="note">Text shown here is a candidate label, not a restored image.</div>
        </div>
      </section>
      <section class="card"><div class="inner">
        <span class="tag">CORRECT FIRST CHOICE</span>
        <h2>Top 3 of {case['tested']} candidates</h2>
        <div class="heading"><span>Ranked candidate</span><span>Physical cost ↓</span></div>
        {rows}
        <p class="note">{case['compatible']} candidates are still physically compatible.<br>
          The first-ranked answer is not the only possible fit.</p>
      </div></section>
    </div>
    <div class="footer">Lower physical cost ranks first. This illustrative fixture does not measure recovery of genuine redactions or unknown grids.</div>
    """


def main():
    from playwright.sync_api import sync_playwright

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--review-project", type=Path, help="Optional project for a real viewer screenshot")
    parser.add_argument("--binary", type=Path, default=ROOT / "target/release/unredact")
    args = parser.parse_args()
    evidence = json.loads((SAMPLES / "evidence.json").read_text())
    with sync_playwright() as playwright, tempfile.TemporaryDirectory() as temp:
        executable = None
        if not Path(playwright.chromium.executable_path).exists():
            executable = shutil.which("google-chrome-stable") or shutil.which("chromium")
        browser = playwright.chromium.launch(executable_path=executable)
        try:
            page = browser.new_page(viewport={"width": 1280, "height": 1000}, device_scale_factor=1)
            for name, markup in [
                ("candidate-ranking.png", opaque_figure(evidence)),
                ("pixelation.png", mosaic_figure(evidence)),
            ]:
                path = Path(temp) / "figure.html"
                path.write_text('<!doctype html><meta charset="utf-8"><style>' + CSS
                                + "</style><main>" + markup + "</main>")
                page.goto(path.as_uri())
                page.evaluate("async () => {await document.fonts.ready; await Promise.all([...document.images].map(i => i.decode()));}")
                page.locator("main").screenshot(path=str(IMAGES / name))
                print("Rendered", IMAGES / name)
            if args.review_project:
                capture_viewer(browser, args.binary, args.review_project)
        finally:
            browser.close()


def capture_viewer(browser, binary, project):
    """Capture the actual app using normal controls; never change its DOM or pixels."""
    original = (project / "project.json").read_bytes()
    server = subprocess.Popen(
        [str(binary.resolve()), "review", "--project", str(project.resolve())],
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
    )
    try:
        if not select.select([server.stdout], [], [], 15)[0]:
            raise RuntimeError("Viewer did not start within 15 seconds")
        line = server.stdout.readline()
        if not line.startswith("Local review: "):
            raise RuntimeError("Viewer did not return a local URL")
        page = browser.new_page(viewport={"width": 1440, "height": 1000}, device_scale_factor=1)
        page.goto(line.removeprefix("Local review: ").strip())
        page.wait_for_function("() => !document.querySelector('#download').disabled")
        page.locator("#show-calibration").uncheck()
        page.locator("#paragraph").check()
        page.locator("#zoom").fill("75")
        page.locator("#zoom").dispatch_event("input")
        page.wait_for_function("() => !document.querySelector('#download').disabled")
        page.evaluate("""async () => {
          await new Promise(resolve => requestAnimationFrame(resolve));
          const v = document.querySelector('#viewport');
          const scale = Number(document.querySelector('#zoom').value) / 100;
          v.scrollLeft = Number(document.querySelector('#x').value) * scale - v.clientWidth / 2;
          v.scrollTop = Number(document.querySelector('#y').value) * scale - v.clientHeight / 2;
          await new Promise(resolve => requestAnimationFrame(resolve));
        }""")
        page.screenshot(path=str(IMAGES / "local-review.png"), full_page=True)
        context = page.locator("#context").inner_text()
        assert "[REDACTED]" in context
        (SAMPLES / "viewer.json").write_text(json.dumps({
            "project_path": str(project),
            "project_sha256": hashlib.sha256(original).hexdigest(),
            "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
            "context_preview": context,
            "zoom_percent": 75,
            "calibration_overlay": False,
            "paragraph_context": True,
            "description": "Screenshot of the running local viewer; no DOM or pixel edits.",
        }, indent=2) + "\n")
        assert (project / "project.json").read_bytes() == original
        print("Captured", IMAGES / "local-review.png")
    finally:
        server.terminate()
        server.communicate(timeout=5)


if __name__ == "__main__":
    main()
