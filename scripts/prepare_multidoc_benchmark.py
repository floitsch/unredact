# Copyright (C) 2026 Toit contributors.
"""Create document-disjoint, paired raster benchmarks from public PDF sources.

Labels/originals live in a controller-only directory. This is an evaluation
fixture generator, never an unredaction engine. OCR labels require review.
"""
import argparse
from collections import Counter
import hashlib
import io
import json
import math
from pathlib import Path
import re
import subprocess
import xml.etree.ElementTree as ET

import numpy as np
from PIL import Image, ImageDraw, ImageFilter
import pytesseract


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def key(seed, *parts):
    return hashlib.sha256(':'.join(map(str, (seed, *parts))).encode()).hexdigest()


def write(path, data):
    path.write_text(json.dumps(data, indent=2) + '\n')


def run(*args):
    return subprocess.check_output(list(map(str, args)), timeout=120, stderr=subprocess.PIPE).decode()


def version(command):
    result = subprocess.run(command,capture_output=True,text=True,check=True,timeout=20)
    return (result.stdout+result.stderr).splitlines()[0]


def rect(box):
    x, y, right, bottom = box
    return dict(x=float(x), y=float(y), w=float(right-x), h=float(bottom-y))


def overlap(a, b):
    return a[0] < b[2] and b[0] < a[2] and a[1] < b[3] and b[1] < a[3]


def native_words(pdf, page, dpi):
    text = run('pdftotext', '-f', page, '-l', page, '-bbox-layout', pdf, '-')
    tree = ET.fromstring(text[text.index('<html'):])
    words = []
    for w in tree.iter():
        if w.tag.rsplit('}', 1)[-1] == 'word':
            words.append((w.text or '', tuple(float(w.attrib[k])*dpi/72
                          for k in ('xMin', 'yMin', 'xMax', 'yMax'))))
    return words


def select_spans(image, ocr, native, spec, plan, page):
    words, rejected = [], Counter()
    for i, text in enumerate(ocr['text']):
        if not text.strip():
            continue
        x, y, w, h = (int(ocr[k][i]) for k in ('left', 'top', 'width', 'height'))
        b = (x, y, x+w, y+h)
        reason = None
        if not re.fullmatch(r'[A-Za-z]{3,18}', text):
            reason = 'token_format'
        elif float(ocr['conf'][i]) < 85:
            reason = 'low_confidence'
        elif min(x, y, image.width-x-w, image.height-y-h) < 15:
            reason = 'page_edge'
        elif spec['kind'] == 'clean' and not any(
                t == text and n[0]-3 <= x+w/2 <= n[2]+3 and n[1]-3 <= y+h/2 <= n[3]+3
                for t, n in native):
            reason = 'native_text_disagrees'
        if reason:
            rejected[reason] += 1
        words.append(dict(index=i, text=text, box=b, eligible=reason is None,
                          line=tuple(ocr[k][i] for k in ('block_num','par_num','line_num'))))
    eligible = {1: [], 2: []}
    for count in (1, 2):
        for i in range(len(words)-count+1):
            group = words[i:i+count]
            if not all(w['eligible'] for w in group) or len({w['line'] for w in group}) != 1:
                continue
            if count == 2:
                a, b = [w['box'] for w in group]
                height = min(a[3]-a[1], b[3]-b[1])
                if not (0 <= b[0]-a[2] <= 2*height and min(a[3],b[3])-max(a[1],b[1]) >= .5*height):
                    continue
            b = (min(w['box'][0] for w in group), min(w['box'][1] for w in group),
                 max(w['box'][2] for w in group), max(w['box'][3] for w in group))
            ink = image.crop(b).point(lambda p: 255 if p < 160 else 0).getbbox()
            if ink is None:
                rejected['no_ink'] += 1
                continue
            bounds = (b[0]+ink[0], b[1]+ink[1], b[0]+ink[2], b[1]+ink[3])
            identity = f"{spec['id']}-p{page}-w{group[0]['index']}-n{count}"
            padding = 1 + int(key(plan['seed'], identity, 'padding')[:8],16) % 6
            mask = (bounds[0]-padding,bounds[1]-2,bounds[2]+padding,bounds[3]+2)
            eligible[count].append(dict(id=identity, text=' '.join(w['text'] for w in group),
                indices=[w['index'] for w in group], ink_box=bounds, box=mask, count=count,
                actual_horizontal_padding=2*padding,
                label_basis='native_text_and_ocr' if spec['kind']=='clean' else 'ocr_pending_visual_review'))
    selected = []
    # Phrases first so single-word picks cannot systematically exhaust them.
    for count in (2,1):
        picked = 0
        for row in sorted(eligible[count], key=lambda r:key(plan['seed'],r['id'])):
            if any(overlap(row['box'], old['box']) for old in selected):
                continue
            # Do not let an expanded mask cover adjacent unlabelled words.
            if any(w['index'] not in row['indices'] and overlap(row['box'],w['box']) for w in words):
                rejected['mask_hits_neighbor'] += 1
                continue
            selected.append(row)
            picked += 1
            if picked == plan['spans_per_page']//2:
                break
    return selected, dict(rejected), len(words)


def pixelate(image, boxes, block):
    """Page-aligned block averages; copy only masked pixels, keep outside unchanged."""
    original = np.asarray(image)
    output = original.copy()
    for box in boxes:
        x0,y0,x1,y1 = map(int,box)
        for y in range(y0//block*block,y1,block):
            for x in range(x0//block*block,x1,block):
                value = int(np.rint(original[y:min(y+block,image.height),x:min(x+block,image.width)].mean()))
                output[max(y,y0):min(y+block,y1),max(x,x0):min(x+block,x1)] = value
    return Image.fromarray(output)


def damage(image, boxes, condition, seed):
    if condition.startswith('pixelate-'):
        return pixelate(image,boxes,int(condition.split('-')[1])), boxes
    result = image.copy()
    draw = ImageDraw.Draw(result)
    for x,y,r,b in boxes:
        draw.rectangle((x,y,r-1,b-1),fill=0)
    if condition == 'opaque-low-resolution':
        result = result.resize((image.width//2,image.height//2),Image.Resampling.BOX).resize(image.size,Image.Resampling.BICUBIC)
    elif condition == 'opaque-jpeg-noise':
        result = result.filter(ImageFilter.GaussianBlur(.7))
        rng = np.random.default_rng(seed)
        result = Image.fromarray(np.clip(np.asarray(result,dtype=float)+rng.normal(0,3,image.size[::-1]),0,255).astype('uint8'))
        buffer = io.BytesIO(); result.save(buffer,format='JPEG',quality=40)
        buffer.seek(0); result = Image.open(buffer).convert('L'); result.load()
    elif condition == 'opaque-subpixel':
        result = result.transform(image.size,Image.Transform.AFFINE,(1,0,-.375,0,1,-.25),Image.Resampling.BICUBIC,fillcolor=255)
        boxes = [(x+.375,y+.25,r+.375,b+.25) for x,y,r,b in boxes]
    elif condition != 'opaque':
        raise ValueError(f'Unknown condition {condition}')
    return result, boxes


def candidates(truth, pool, count, seed):
    rng = np.random.default_rng(seed)
    eligible = [w for w in pool if w != truth]
    chosen = [eligible[int(i)] for i in rng.choice(len(eligible),count-1,replace=False)] + [truth]
    return [dict(id=key('candidate',text),canonical_name=text,surfaces=[dict(text=text,kind='canonical')],
                 source_ids=[],context_score=None) for text in sorted(chosen)]


def target_evidence(condition, box):
    if condition.startswith('pixelate-'):
        # The legacy scorer renders inside a floor/ceil-rounded target crop.
        # Estimate positions from the mask alone, in that crop's coordinates.
        # Never supply original glyph positions or the true pixelation block.
        return dict(kind='mosaic_auto', min_block=4, max_block=20,
                    origin_x=box[0]-math.floor(box[0]),
                    baseline_y=box[3]-2.-math.floor(box[1]), noise_sigma=8.)
    return dict(kind='opaque', padding_min=0., padding_max=12., measurement_sigma=1.5)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--plan',type=Path,required=True)
    parser.add_argument('--sources',type=Path,required=True)
    parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--vocabulary',type=Path,default=Path('baseline-alex/unredact/data/nouns.txt'))
    args = parser.parse_args()
    plan = json.loads(args.plan.read_text()); sources = json.loads(args.sources.read_text())
    if plan['spans_per_page'] <= 0 or plan['spans_per_page'] % 2:
        raise ValueError('spans_per_page must be positive and even')
    if not (1 <= plan['pages_per_document'] <= plan['max_pages_examined'] <= 100 and 72 <= plan['dpi'] <= 600):
        raise ValueError('Invalid page budget or DPI')
    groups = {}
    for d in plan['documents']:
        if groups.setdefault(d['group'],d['split']) != d['split']:
            raise ValueError('Source group crosses development/test split')
    if len({d['id'] for d in plan['documents']}) != len(plan['documents']):
        raise ValueError('Duplicate document ID')
    out = args.output.resolve(); out.mkdir(parents=True,exist_ok=False)
    private = out/'controller'; private.mkdir(); (out/'images').mkdir()
    write(out/'plan.json',plan)
    pool = sorted({w.strip() for w in args.vocabulary.read_text().splitlines() if re.fullmatch('[A-Za-z]{3,18}',w.strip())})
    # Mix word and phrase distractors without learning from test document text.
    pool = sorted(set(pool + [pool[i]+' '+pool[(i*31+17)%len(pool)] for i in range(len(pool))]))
    suites = {}; metadata = []; documents = []; seen_pdf = set(); seen_pixels = set()
    for spec in plan['documents']:
        source = next(s for s in sources if s['id']==spec['id'])
        record = dict(**spec,pages=[],errors=[]); documents.append(record)
        if source.get('error'):
            record['errors'].append(source['error']); continue
        pdf = Path(source['path'])
        if sha(pdf) != source['sha256'] or source['sha256'] in seen_pdf:
            raise ValueError('Changed or duplicated source PDF')
        seen_pdf.add(source['sha256']); record['sha256'] = source['sha256']
        count = int(re.search(r'^Pages:\s+(\d+)',run('pdfinfo',pdf),re.M)[1])
        selected_pages = 0
        ordering = sorted(range(2,count+1),key=lambda p:key(plan['seed'],spec['id'],p))[:plan['max_pages_examined']]
        for number in ordering:
            page_record = dict(page=number); record['pages'].append(page_record)
            prefix = private/f"{spec['id']}-p{number}"
            try:
                run('pdftoppm','-f',number,'-l',number,'-singlefile','-r',plan['dpi'],'-gray',pdf,prefix)
                image = Image.open(str(prefix)+'.pgm').convert('L')
                pixel_sha = hashlib.sha256(image.tobytes()).hexdigest()
                if pixel_sha in seen_pixels:
                    raise ValueError('Duplicate rendered page')
                seen_pixels.add(pixel_sha)
                if image.width*image.height > 25_000_000:
                    raise ValueError('Page exceeds pixel budget')
                image_list = run('pdfimages','-f',number,'-l',number,'-list',pdf)
                (private/f'{prefix.name}-images.txt').write_text(image_list)
                # Largest embedded image footprint, used as an explicit structural check.
                coverage=[]
                for line in image_list.splitlines()[2:]:
                    columns=line.split()
                    if len(columns)>=14 and columns[2]=='image':
                        try:
                            xp,yp=float(columns[12]),float(columns[13])
                            if xp>0 and yp>0:
                                coverage.append((float(columns[3])*plan['dpi']/xp)*(float(columns[4])*plan['dpi']/yp)/(image.width*image.height))
                        except ValueError:
                            pass
                page_record['largest_embedded_image_coverage']=max(coverage,default=0.)
                if spec['kind']=='clean' and max(coverage,default=0.)>=.75:
                    raise ValueError('Declared clean source page is dominated by raster image')
                if spec['kind']=='scan' and max(coverage,default=0.)<.75:
                    raise ValueError('Declared scan page lacks a page-sized embedded image')
                ocr = pytesseract.image_to_data(image,config='--psm 3',output_type=pytesseract.Output.DICT,timeout=120)
                write(private/f'{prefix.name}-ocr.json',ocr)
                native = native_words(pdf,number,plan['dpi']) if spec['kind']=='clean' else []
                selected, rejects, word_count = select_spans(image,ocr,native,spec,plan,number)
                page_record.update(ocr_words=word_count,rejections=rejects,selected_count=len(selected),pixel_sha256=pixel_sha)
                if word_count < 60 or len(selected) != plan['spans_per_page']:
                    page_record['status']='insufficient_eligible_text'; continue
                page_record['status']='selected'; selected_pages += 1
                sheet = Image.new('L',(1100,len(selected)*115),255)
                for j,row in enumerate(selected):
                    row.update(document=spec['id'],source_group=spec['group'],split=spec['split'],kind=spec['kind'],page=number,original_pixel_sha256=pixel_sha,review='pending')
                    x,y,r,b = row['box']; crop=image.crop((max(0,x-100),max(0,y-12),min(image.width,r+100),min(image.height,b+12)))
                    crop.thumbnail((1080,82)); sheet.paste(crop,(10,j*115+28))
                    ImageDraw.Draw(sheet).text((10,j*115+6),f"{row['id']} | {row['text']}",fill=0)
                    metadata.append(row)
                sheet.save(private/f'{prefix.name}-review.png')
                boxes = [r['box'] for r in selected]
                for condition in plan['conditions']:
                    seed = int(key(plan['seed'],spec['id'],number,condition)[:8],16)
                    damaged, masks = damage(image,boxes,condition,seed)
                    name=f'{prefix.name}-{condition}.pgm'; path=out/'images'/name; damaged.save(path)
                    page=dict(id=prefix.name,image=str(path.relative_to(out)),image_sha256=sha(path),dpi=plan['dpi'],redaction_regions=list(map(rect,masks)))
                    for row,box in zip(selected,masks):
                        vocab = candidates(row['text'],pool,plan['candidate_count'],int(key(plan['seed'],row['id'],'vocab')[:8],16))
                        evidence = target_evidence(condition, box)
                        target=dict(id=row['id'],page=page['id'],bounds=rect(box),surface_kind='canonical',evidence=evidence,left_context='',right_context='',dependence_key=None)
                        for availability in ('present','absent'):
                            identity=f"{row['id']}-{condition}-{availability}"
                            case=dict(id=identity,truth_id=key('candidate',row['text']),input=dict(schema_version=1,pages=[page],targets=[target],candidates=[c for c in vocab if availability=='present' or c['canonical_name']!=row['text']]))
                            bucket=f"{spec['split']}-{condition}-{availability}"
                            suites.setdefault(bucket,[]).append(case)
                            row.setdefault('cases',[]).append(dict(id=identity,suite=bucket,condition=condition,availability=availability))
                print(spec['id'],number,'selected',len(selected),flush=True)
            except (subprocess.CalledProcessError,subprocess.TimeoutExpired,RuntimeError,ValueError,OSError,ET.ParseError) as exc:
                if page_record.get('status') == 'selected':
                    raise
                page_record.update(status='error',error=str(exc)); print(spec['id'],number,'ERROR',str(exc),flush=True)
            if selected_pages == plan['pages_per_document']:
                break
        record['selected_pages']=selected_pages
        if selected_pages < plan['pages_per_document']:
            record['errors'].append('Page quota unmet; no substitutions outside fixed page budget')
    for name,cases in suites.items():
        write(out/f'{name}.json',dict(schema_version=1,track='candidate_ranking_from_images',description='Document-disjoint controlled masks; supplied locations; hidden padding0..12; mixed300word/phrase vocabulary; labels pending review. Pixelation origin/baseline approximated from mask only.',cases=cases))
    write(private/'labels.json',dict(labels=metadata,documents=documents))
    write(out/'manifest.json',dict(schema_version=1,plan_sha256=sha(args.plan),sources_sha256=sha(args.sources),generator_sha256=sha(__file__),vocabulary_sha256=sha(args.vocabulary),documents=documents,unique_spans=len(metadata),cases=sum(map(len,suites.values())),suites={n:dict(cases=len(c),sha256=sha(out/f'{n}.json')) for n,c in suites.items()},label_review='pending; do not score as verified ground truth',runtime={p:version(cmd) for p,cmd in [('poppler',['pdftoppm','-v']),('tesseract',['tesseract','--version'])]}))
    print('Unique spans:',len(metadata),'cases:',sum(map(len,suites.values())),flush=True)


if __name__=='__main__':
    main()
