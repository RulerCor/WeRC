"""Compose WeRC branding over the supplied portrait without repainting it."""
from pathlib import Path
import hashlib
import json
from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parent.parent
ASSETS = ROOT / 'assets'
source = ASSETS / 'app-icon-original.jpg'
portrait = Image.open(source).convert('RGBA')
side = max(portrait.size)
canvas = Image.new('RGBA', (side, side), (22, 22, 22, 255))
offset = ((side-portrait.width)//2, (side-portrait.height)//2)
canvas.alpha_composite(portrait, offset)
draw = ImageDraw.Draw(canvas)
ink, yellow, paper = '#161616', '#FFFA00', '#F4F4F2'
def box(coords, color):
    draw.rectangle(tuple(round(v*side/1050) for v in coords), fill=color)
def polygon(coords, color):
    draw.polygon([(round(x*side/1050), round(y*side/1050)) for x,y in coords], fill=color)

# The portrait and expression stay untouched; branding occupies the corners.
box((0,0,1049,11), ink)
box((0,1038,1049,1049), ink)
box((0,0,11,1049), ink)
box((1038,0,1049,1049), ink)
polygon([(28,28),(382,28),(382,126),(356,152),(28,152)], yellow)
font = ImageFont.truetype(str(ROOT/'apps/desktop/src/assets/fonts/Saira.ttf'), round(100*side/1050))
font.set_variation_by_axes([750,100])
draw.text((round(48*side/1050),round(15*side/1050)), 'WeRC', font=font, fill=ink, stroke_width=1)
box((28,168,154,179), paper)
box((164,168,214,179), yellow)
box((224,168,246,179), ink)
box((990,28,1021,144), yellow)
box((990,156,1021,179), paper)
polygon([(930,1018),(1018,930),(1018,1018)], ink)
polygon([(970,1018),(1018,970),(1018,1018)], yellow)
output = ASSETS/'app-icon.png'
canvas.save(output)
canvas.save(ASSETS/'app-icon-endfield.png')
manifest = {'format':1,'method':'deterministic_original_portrait_composite',
 'source':source.name,'source_sha256':hashlib.sha256(source.read_bytes()).hexdigest(),
 'output_sha256':hashlib.sha256(output.read_bytes()).hexdigest(),
 'dimensions':list(canvas.size),'source_offset':list(offset),
 'preserved_rectangle':[20,200,970,920],
 'branding':{'text':'WeRC','palette':[ink,yellow,paper],'placement':'upper_left'}}
(ASSETS/'app-icon-branding.json').write_text(json.dumps(manifest,indent=2)+'\n',encoding='utf-8')
print(json.dumps(manifest))
