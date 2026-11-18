"""Published price snapshots and conservative, auditable token arithmetic."""

import math
import re

PRICING_URL = 'https://developers.openai.com/api/docs/pricing#image-generation'
PRICING_MARKDOWN = 'https://developers.openai.com/api/docs/pricing.md'
GUIDE_URL = 'https://developers.openai.com/api/docs/guides/image-generation#calculating-costs'
BATCH_URL = 'https://developers.openai.com/api/docs/guides/batch'
CACHE_URL = 'https://developers.openai.com/api/docs/guides/image-generation#cached-input-pricing'
VERIFIED = '2026-11-18'
MODELS = ['gpt-image-2.6-sunburst', 'gpt-image-2.6-flare',
          'gpt-image-2.5-sunburst', 'gpt-image-2.5-flare']
RATE_LABELS = {'text_input': 'Text input', 'cached_text_input': 'Cached text input',
               'image_input': 'Image input', 'cached_image_input': 'Cached image input',
               'image_output': 'Image output'}
STANDARD = dict(text_input=5.0, cached_text_input=1.25, image_input=8.0,
                cached_image_input=2.0, image_output=30.0)
BATCH = dict(text_input=2.5, cached_text_input=0.625, image_input=4.0,
             cached_image_input=1.0, image_output=15.0)


def default_catalog():
    return {model: {'Standard': STANDARD.copy(), 'Batch': BATCH.copy()} for model in MODELS}


def parse_official_prices(markdown):
    """Read only the image tables. Refuse partial results or a changed page format."""
    images = re.split(r'^Images\s*$', markdown, flags=re.M)
    if len(images) != 2:
        raise ValueError('Could not identify the official image pricing section. Use the source link to check rates.')
    section = re.split(r'^Audio\s*$', images[1], maxsplit=1, flags=re.M)[0]
    catalog = {model: {} for model in MODELS}
    mode = None
    for line in section.splitlines():
        if line.strip() in ('Standard', 'Batch'):
            mode = line.strip()
        if mode is None or not line.startswith('|'):
            continue
        cells = [cell.strip() for cell in line.strip().strip('|').split('|')]
        if len(cells) != 5 or cells[0] not in catalog:
            continue
        model, modality, input_rate, output_rate, cached = cells
        def price(value):
            value = float(value.replace('$', '').replace(',', ''))
            if not math.isfinite(value) or value < 0:
                raise ValueError('Invalid published rate')
            return value
        rates = catalog[model].setdefault(mode, {})
        if modality == 'Text':
            rates.update(text_input=price(input_rate), cached_text_input=price(cached))
        elif modality == 'Image':
            rates.update(image_input=price(input_rate), image_output=price(output_rate),
                         cached_image_input=price(cached))
    if any(set(catalog[m].get(mode, {})) != set(RATE_LABELS)
           for m in MODELS for mode in ('Standard', 'Batch')):
        raise ValueError('The official page no longer contains all expected rates. Existing rates were kept.')
    return catalog


def token_count(value):
    return value if isinstance(value, int) and not isinstance(value, bool) and value >= 0 else None


def estimate(usage, rates, *, cache_eligible=False, uplift_pct=0.0):
    """Missing quantities remain unknown. Output tokens are request totals, never multiplied by n."""
    if not usage:
        return None
    details = usage.get('input_tokens_details') or {}
    text = token_count(details.get('text_tokens'))
    image = token_count(details.get('image_tokens'))
    output = token_count(usage.get('output_tokens'))
    notes = []
    if usage.get('input_tokens') == 0:
        text = 0 if text is None else text
        image = 0 if image is None else image
    total_input = token_count(usage.get('input_tokens'))
    if text is not None and image is not None and total_input is not None and text + image != total_input:
        notes.append('Reported input totals do not match the modality split; input costs are unavailable.')
        text = image = None
    cached = details.get('cached_tokens_details') or {}
    ct = token_count(cached.get('text_tokens'))
    ci = token_count(cached.get('image_tokens'))
    cache_total = token_count(details.get('cached_tokens'))
    if cache_eligible:
        if ct is None and ci is None and cache_total in (None, 0):
            ct = ci = 0
            notes.append('No cache hits reported; estimate uses uncached input rates.')
        elif ct is None or ci is None or (cache_total is not None and ct + ci != cache_total):
            ct = ci = None
            notes.append('Cached input modality split unavailable; input costs cannot be calculated reliably.')
    else:
        ct = ci = 0
        if cache_total or cached:
            notes.append('Cached discounts are not applied to this direct Images API request; see caching eligibility.')
    if (text is not None and ct is not None and ct > text) or (image is not None and ci is not None and ci > image):
        ct = ci = None
        notes.append('Reported cached tokens exceed input tokens; input costs are unavailable.')
    quantities = {
        'text_input': text - ct if text is not None and ct is not None else None,
        'cached_text_input': ct,
        'image_input': image - ci if image is not None and ci is not None else None,
        'cached_image_input': ci,
        'image_output': output,
    }
    lines = []
    for name, tokens in quantities.items():
        rate = rates.get(name)
        cost = tokens * rate / 1e6 if tokens is not None and rate is not None else (0.0 if tokens == 0 else None)
        lines.append({'component': RATE_LABELS[name], 'key': name, 'tokens': tokens,
                      'rate': rate, 'usd': cost})
    complete = all(line['usd'] is not None for line in lines)
    subtotal = sum(line['usd'] for line in lines if line['usd'] is not None)
    uplift = subtotal * uplift_pct / 100
    if not complete:
        notes.append('Some token quantities or rates are missing. Known components are shown; the total is unavailable.')
    by_key = {line['key']: line['usd'] for line in lines}
    def combined(a, b):
        return by_key[a] + by_key[b] if by_key[a] is not None and by_key[b] is not None else None
    return {'text': combined('text_input', 'cached_text_input'),
            'image': combined('image_input', 'cached_image_input'), 'output': by_key['image_output'],
            'lines': lines, 'subtotal': subtotal if complete else None,
            'known_subtotal': subtotal, 'uplift_pct': uplift_pct, 'uplift': uplift,
            'known_total': subtotal + uplift, 'total': subtotal + uplift if complete else None,
            'warning': ' '.join(notes) or None}
