# Copyright (C) 2026 Toit contributors.
"""Run Alex-Gilbert/unredact's current JS/OCR/WASM path in a fresh browser.

Uses original OCR, analyzeRedaction, detectFontMasked, measureWidths, and WASM solve.
Only private-function exports and local asset URLs are adapted. Input geometry and
vocabulary replace UI selection/upload. Does not add joint inference or AI ranking.
"""
import argparse
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import shutil
import sys
from threading import Thread

from external_common import dump, groups, single_target, unsupported


def extract_function(path, signature):
    source = path.read_text()
    start = source.index(signature)
    end = source.index("\n}\n", start) + 3
    return "export " + source[start:end]


def prepare_site(repo, pkg, vendor, site, image):
    from PIL import Image
    site.mkdir()
    for source in (repo / "unredact/static").glob("*.js"):
        shutil.copyfile(source, site / source.name)
    shutil.copytree(repo / "unredact/static/fonts", site / "fonts")
    shutil.copytree(pkg, site / "pkg")
    (site / "vendor").symlink_to(vendor, target_is_directory=True)
    Image.open(image).convert("RGB").save(site / "input.png")
    (site / "index.html").write_text('<!doctype html><canvas id="overlay-canvas"></canvas>')
    original = (site / "ocr.js").read_text()
    modified = original.replace(
        "https://cdn.jsdelivr.net/npm/tesseract.js@5/dist/tesseract.esm.min.js",
        "/vendor/tesseract.js/dist/tesseract.esm.min.js")
    marker = "Tesseract.createWorker('eng', 1, {"
    if modified == original or modified.count(marker) != 1:
        raise ValueError("Upstream OCR initialization changed; review adapter")
    modified = modified.replace(marker, marker + "\n"
        "workerPath: '/vendor/tesseract.js/dist/worker.min.js',\n"
        "corePath: '/vendor/tesseract.js-core',\n"
        "langPath: '/vendor/@tesseract.js-data/eng/4.0.0',\n")
    (site / "ocr.js").write_text(modified)
    # These functions have no top-level unindented closing braces inside their
    # bodies. Extract their original bodies; fail if upstream layout changes.
    analysis = extract_function(repo / "unredact/static/main.js", "async function analyzeRedaction(")
    measurement = extract_function(repo / "unredact/static/solver.js", "function measureWidths(")
    (site / "baseline-entry.js").write_text(
        "import { state } from './state.js';\n"
        "import { detectFontMasked } from './font_detect.js';\n"
        + analysis + "\n" + measurement)


BROWSER_RUN = """async ({target, entries, tolerance}) => {
    const {initWasm, solve} = await import('/wasm.js');
    const {ocrPage, terminateOcr} = await import('/ocr.js');
    const {state} = await import('/state.js');
    const {analyzeRedaction, measureWidths} = await import('/baseline-entry.js');
    await initWasm();
    const manifest = await (await fetch('/fonts/manifest.json')).json();
    state.fonts = [];
    for (const f of manifest.fonts) {
        const face = await new FontFace(f.name, `url(/fonts/${f.file})`).load();
        document.fonts.add(face);
        state.fonts.push({...f, available:true});
    }
    const img = new Image();
    img.src = '/input.png';
    await img.decode();
    const canvas = new OffscreenCanvas(img.width, img.height);
    const ctx = canvas.getContext('2d');
    ctx.drawImage(img, 0, 0);
    const pixels = ctx.getImageData(0, 0, img.width, img.height);
    const lines = await ocrPage(await canvas.convertToBlob());
    const analysis = await analyzeRedaction(pixels, lines, target.bounds, null);
    if (!analysis) {
        await terminateOcr();
        return {analysis:null, lines, results:[], measurements:[]};
    }
    const left = analysis.segments[0].text;
    const right = analysis.segments[1].text;
    const measurements = measureWidths(entries.map(x => x[1]), analysis.font.name,
        analysis.font.size, left.length ? left[left.length - 1] : '', right[0] || '');
    const targetWidth = target.evidence.kind === 'width' ? target.evidence.width : analysis.gap.w;
    const results = solve({entries:measurements, target_width:targetWidth, tolerance,
        known_start:'', known_end:'', mode:'name'});
    await terminateOcr();
    return {analysis, lines, results, measurements, targetWidth, tolerance};
}"""


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repository", type=Path, required=True)
    parser.add_argument("--wasm-package", type=Path, required=True)
    parser.add_argument("--browser-deps", type=Path, required=True)
    parser.add_argument("--browser", type=Path, required=True)
    parser.add_argument("--tolerance", type=float, default=1.1,
                        help="Effective upstream default is UI 1px + 0.1px")
    args = parser.parse_args()
    request = json.load(sys.stdin)
    try:
        target, page, entries = single_target(request["input"])
    except ValueError as error:
        print(json.dumps(unsupported(str(error))))
        return
    work = Path(request["work_dir"])
    work.mkdir(parents=True, exist_ok=False)
    site = work / "site"
    prepare_site(args.repository.resolve(), args.wasm_package.resolve(),
                 args.browser_deps.resolve(), site, page["image"])

    class QuietHandler(SimpleHTTPRequestHandler):
        def log_message(self, *_):
            pass

    server = ThreadingHTTPServer(("127.0.0.1", 0), partial(QuietHandler, directory=str(site)))
    thread = Thread(target=server.serve_forever, daemon=True)
    thread.start()
    messages = []
    try:
        from playwright.sync_api import sync_playwright
        with sync_playwright() as playwright:
            with playwright.chromium.launch(executable_path=str(args.browser.resolve()), headless=True) as browser:
                context = browser.new_context()
                origin = f"http://127.0.0.1:{server.server_port}"
                context.route("**/*", lambda route: route.continue_()
                              if route.request.url.startswith(origin + "/") else route.abort())
                tab = context.new_page()
                tab.on("console", lambda message: messages.append(message.text))
                tab.on("pageerror", lambda error: messages.append(str(error)))
                tab.goto(origin + "/index.html")
                result = tab.evaluate(BROWSER_RUN, {"target": target, "entries": entries,
                                                   "tolerance": args.tolerance})
                result["browser_version"] = browser.version
    finally:
        server.shutdown()
        server.server_close()
        thread.join()
        dump(work / "browser-log.json", messages)
    dump(work / "native-result.json", result)
    if result["analysis"] is None:
        print(json.dumps({**unsupported("Upstream found no OCR line overlapping the target"), "status": "error"}))
        return
    errors = {r["text"]: r["error"] for r in result["results"]}
    scores = [(identity, errors[text]) for identity, text in entries if text in errors]
    print(json.dumps({"schema_version": 1, "status": "ok", "reason": None,
                      "ranked_groups": groups(scores), "ranking_complete": len(scores) == len(entries),
                      "compatible": [identity for identity, _ in scores]}))


if __name__ == "__main__":
    main()
