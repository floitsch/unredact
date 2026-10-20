# Copyright (C) 2026 Toit contributors.
"""Browser interaction smoke test; requires Python's optional playwright package.

Run after `cargo build`: python3 scripts/test_local_review.py
"""
import argparse
import hashlib
import json
from pathlib import Path
import select
import shutil
import subprocess
import tempfile


def fixture(directory):
    width, height = 900, 700
    pixels = bytearray([255]) * (width * height)
    for x, y, w, h in [(30, 30, 70, 20), (210, 200, 120, 30)]:
        for row in range(y, y + h):
            pixels[row * width + x:row * width + x + w] = bytes(w)
    (directory / 'page.pgm').write_bytes(f'P5\n{width} {height}\n255\n'.encode() + pixels)
    page_id = 'page </script><script>window.unredactInjected=true</script>'
    word = lambda text, x, y, w: dict(text=text, bounds=dict(x=x, y=y, w=w, h=20),
                                     confidence=99, block='body', line='line')
    project = dict(schema_version=1, pages=[dict(
        id=page_id, source='fixture.pdf', source_sha256='source', page_number=1,
        dpi=200, width=width, height=height, image='page.pgm',
        image_sha256=hashlib.sha256(pixels).hexdigest(), likely_scan=True,
        image_coverage=1, native_chars=[], proposed_redactions=[dict(x=210, y=200, w=120, h=30)],
        words=[word('Training', 30, 30, 70), word('Visible', 130, 200, 70), word('context', 340, 200, 90)],
    )], profiles=[dict(
        id='body', page=page_id, block=None, training_texts=['Training'], validation_texts=[], audit_texts=[],
        fits=[dict(font='unused.ttf', kern=True, size=12, tracking=0, ink_bias=0, corrections={},
                   validation_rmse=1, validation_max_error=2, training_words=20, validation_words=10,
                   width_metric='ink')],
    )], targets=[])
    (directory / 'project.json').write_text(json.dumps(project))


def run(binary, screenshot):
    from playwright.sync_api import sync_playwright
    with tempfile.TemporaryDirectory(prefix='unredact-browser-') as temporary:
        directory = Path(temporary)
        fixture(directory)
        original = (directory / 'project.json').read_bytes()
        process = subprocess.Popen([str(binary.resolve()), 'review', '--project', str(directory)],
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        try:
            assert select.select([process.stdout], [], [], 15)[0], 'Review server did not start'
            line = process.stdout.readline()
            assert line.startswith('Local review: '), (line, process.stderr.read())
            url = line.removeprefix('Local review: ').strip()
            with sync_playwright() as playwright:
                executable = None
                if not Path(playwright.chromium.executable_path).exists():
                    executable = shutil.which('google-chrome-stable') or shutil.which('chromium')
                browser = playwright.chromium.launch(executable_path=executable)
                page = browser.new_page(viewport=dict(width=1440, height=1100), accept_downloads=True)
                errors = []
                page.on('pageerror', lambda error: errors.append(str(error)))
                page.goto(url)
                page.wait_for_function("() => document.querySelector('#validation').textContent.includes('Valid target')")
                assert page.locator('#context').inner_text() == 'Visible [REDACTED] context'
                assert page.locator('#existing').input_value() == 'proposal:0'
                assert page.evaluate('() => window.unredactInjected') is None
                page.locator('#zoom').fill('100')
                page.locator('#zoom').dispatch_event('input')
                box = page.locator('#page').bounding_box()
                # Move the draft, then resize the bottom-right corner.
                page.mouse.move(box['x'] + 260, box['y'] + 215)
                page.mouse.down()
                page.mouse.move(box['x'] + 280, box['y'] + 235, steps=4)
                page.mouse.up()
                assert float(page.locator('#x').input_value()) == 230
                assert float(page.locator('#y').input_value()) == 220
                page.mouse.move(box['x'] + 350, box['y'] + 250)
                page.mouse.down()
                page.mouse.move(box['x'] + 365, box['y'] + 260, steps=4)
                page.mouse.up()
                assert float(page.locator('#w').input_value()) == 135
                assert float(page.locator('#h').input_value()) == 40
                # Draw a fresh rectangle; group/profile/evidence form a full spec.
                page.locator('#draw').click()
                page.mouse.move(box['x'] + 210, box['y'] + 200)
                page.mouse.down()
                page.mouse.move(box['x'] + 330, box['y'] + 230, steps=5)
                page.mouse.up()
                assert float(page.locator('#x').input_value()) == 210
                assert float(page.locator('#w').input_value()) == 120
                page.locator('#download').wait_for(state='visible')
                page.wait_for_function("() => !document.querySelector('#download').disabled")
                with page.expect_download() as download_info:
                    page.locator('#download').click()
                exported = json.loads(Path(download_info.value.path()).read_text())
                assert exported['bounds'] == dict(x=210, y=200, w=120, h=30)
                assert exported['profile'] == 'body'
                assert (directory / 'project.json').read_bytes() == original
                # Downloaded target is accepted by the normal CLI registration path.
                target_file = directory / 'downloaded.json'
                target_file.write_text(json.dumps(exported))
                subprocess.run([str(binary.resolve()), 'target', '--project', str(directory), '--spec', str(target_file)], check=True)
                # Calibration overlap blocks export with an actionable error.
                for key, value in dict(x=30, y=30, w=70, h=20).items():
                    page.locator('#' + key).fill(str(value))
                page.wait_for_function("() => document.querySelector('#validation').textContent.includes('calibration/validation')")
                assert page.locator('#download').is_disabled()
                page.locator('#evidence').fill('{invalid')
                assert 'Invalid evidence JSON' in page.locator('#validation').inner_text()
                # Supplied strings remain text, even if they contain HTML syntax.
                page.locator('#evidence').fill(json.dumps(exported['evidence']))
                for key, value in exported['bounds'].items():
                    page.locator('#' + key).fill(str(value))
                page.locator('summary').first.click()
                page.locator('#left').fill('<img src=x onerror="window.unredactInjected=true">')
                page.wait_for_function("() => document.querySelector('#context').textContent.includes('<img')")
                assert page.evaluate('() => window.unredactInjected') is None
                page.locator('#paragraph').check()
                page.wait_for_function("() => !document.querySelector('#download').disabled")
                assert 'supplied' in page.locator('#sources').inner_text()
                if screenshot:
                    page.screenshot(path=str(screenshot), full_page=True)
                assert not errors, errors
                browser.close()
            print('Local review browser smoke test passed: draw, move, resize, validation, context, download, registration, HTML escaping.')
        finally:
            process.terminate()
            process.communicate(timeout=5)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=Path('target/debug/unredact'))
    parser.add_argument('--screenshot', type=Path)
    args = parser.parse_args()
    run(args.binary, args.screenshot)
