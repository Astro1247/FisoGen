import json
import os
import re
import time
from pathlib import Path

import pandas as pd
import requests
import streamlit as st

from pricing import (BATCH_URL, CACHE_URL, GUIDE_URL, MODELS, PRICING_MARKDOWN,
                     PRICING_URL, RATE_LABELS, VERIFIED, default_catalog,
                     estimate, parse_official_prices)
from studio_api import (TERMINAL, api_request, append_record, draft_entries,
                        image_body, load_jobs, make_record, now, read_history,
                        refresh_batch, save_job, serialize_batch, submit_batch)

ROOT = Path(__file__).parent
OUT = ROOT / 'outputs'
OUT.mkdir(exist_ok=True)
HISTORY = ROOT / 'history.jsonl'
JOBS = ROOT / 'batches'

st.set_page_config(page_title='FisoGen', page_icon='🎨', layout='wide')
st.markdown('''<style>
.block-container{max-width:1400px;padding-top:2rem}
[data-testid="stMetric"]{background:rgba(128,128,128,.07);border-radius:14px;padding:14px}
[data-testid="stSidebar"]{min-width:300px}
.stButton button[kind="primary"]{font-weight:700}
</style>''', unsafe_allow_html=True)


def money(value):
    return f'${value:,.6f}' if value is not None else 'Unavailable'


def source_links():
    links = [('Official image prices ↗', PRICING_URL), ('Image cost calculator ↗', GUIDE_URL),
             ('Batch guide ↗', BATCH_URL), ('Account usage ↗', 'https://platform.openai.com/usage')]
    for start in (0, 2):
        for col, (label, url) in zip(st.columns(2), links[start:start + 2]):
            col.link_button(label, url, width='stretch')


def show_cost(record, *, prefix, show_images=False):
    cost = record.get('cost')
    if record.get('error'):
        st.error(f'Request error: {record["error"]}')
    for error in record.get('output_errors', []):
        st.warning(error)
    count = record.get('returned_images', len(record.get('files', [])))
    a, b, c = st.columns(3)
    a.metric('Estimated request cost', money(cost.get('total')) if cost else 'Unavailable')
    b.metric('Average / returned image', money(cost['total'] / count)
             if cost and cost.get('total') is not None and count else 'Unavailable')
    c.metric('Returned / requested images', f'{count} / {record.get("requested", "—")}')
    st.caption(f'{record.get("mode", "Standard")} · {record["model"]} · '
               f'{record.get("pricing_basis", "Legacy rate snapshot")} · '
               f'prices checked {record.get("pricing_checked", "not recorded")} · USD')
    if cost:
        if cost.get('lines'):
            st.dataframe([{'Component': line['component'], 'Reported tokens': line['tokens'],
                           'USD / 1M tokens': line['rate'], 'Cost USD': line['usd'],
                           'Calculation': f'{line["tokens"]:,} × {line["rate"]:g} / 1,000,000'
                           if line['tokens'] is not None and line['rate'] is not None else 'Unavailable'}
                          for line in cost['lines']], hide_index=True, width='stretch',
                         column_config={'Cost USD': st.column_config.NumberColumn(format='$%.6f')})
            st.caption(f'Known token costs: {money(cost["known_subtotal"])} · '
                       f'custom adjustment ({cost.get("uplift_pct", 0):g}%): {money(cost.get("uplift"))}')
            if cost.get('total') is None:
                st.info(f'Known components only: {money(cost["known_total"])}. This is not a complete request total.')
        else:
            st.table([{'Component': name, 'USD': money(cost.get(name))} for name in ('text', 'image', 'output', 'total')])
        if cost.get('warning'):
            st.warning(cost['warning'])
        comparison = record.get('comparison_rates')
        if comparison and record.get('usage'):
            options = {mode: estimate(record['usage'], values,
                                      cache_eligible=record['model'].startswith('gpt-image-2.6'),
                                      uplift_pct=record.get('uplift_pct', 0))
                       for mode, values in comparison.items()}
            if all(result and result['total'] is not None for result in options.values()):
                standard, batch = options['Standard']['total'], options['Batch']['total']
                st.info(f'Same reported tokens at saved rates: Standard {money(standard)} · Batch {money(batch)} · '
                        f'difference {money(standard - batch)}. Comparison only; token use can vary between requests.')
    else:
        st.warning('No usable API usage was returned. The cost is unknown, not zero.')
    with st.expander('All request details, rate snapshot & raw usage'):
        st.write({name: record.get(name) for name in
                  ('time', 'mode', 'model', 'quality', 'size', 'background', 'format', 'complexity',
                   'references', 'elapsed_s', 'response_id', 'request_id', 'batch_id', 'custom_id')})
        st.write('Prompt', record.get('prompt', ''))
        st.write('Saved rates (USD / 1M tokens)')
        st.json(record.get('rates', {}))
        st.write('Raw API usage')
        st.json(record.get('usage') or {})
        st.markdown(f'[Pricing source]({record.get("pricing_source", PRICING_URL)})')
        st.download_button('Download complete cost details JSON',
                           json.dumps(record, ensure_ascii=False, indent=2).encode('utf-8'),
                           'image_cost_details.json', 'application/json', key=prefix + '_json')
    if show_images:
        for filename in record.get('files', []):
            path = OUT / Path(filename).name
            if not path.exists():
                st.warning(f'Saved image is missing: {filename}')
                continue
            raw = path.read_bytes()
            st.image(raw, width='stretch')
            mime = 'image/jpeg' if record['format'] == 'jpeg' else 'image/' + record['format']
            st.download_button('Download image', raw, filename, mime, key=prefix + filename)


if 'catalog' not in st.session_state:
    st.session_state.catalog = default_catalog()
    st.session_state.pricing_checked = VERIFIED

with st.sidebar:
    st.header('⚙️ FisoGen')
    key = st.text_input('OpenAI API key', value=os.getenv('OPENAI_API_KEY', ''), type='password',
                        help='Used by this local server. Never saved in history or Batch job files.')
    st.caption('Requests go directly to api.openai.com.')
    model = st.selectbox('Model', MODELS)
    mode = st.radio('Processing', ['Standard', 'Batch'], horizontal=True,
                    help='Standard returns images immediately. Batch queues requests with a 24-hour completion window.')
    if mode == 'Batch':
        st.success('Batch · discounted published token rates')
        st.caption('Submit here, then refresh and collect in Batch jobs.')
    st.divider()
    st.subheader('Pricing')
    st.link_button('Open official image pricing ↗', PRICING_URL, width='stretch')
    st.caption(f'Published snapshot checked: {st.session_state.pricing_checked}')
    if st.button('Refresh official prices', width='stretch'):
        try:
            response = requests.get(PRICING_MARKDOWN, timeout=20)
            response.raise_for_status()
            fresh = parse_official_prices(response.text)
            st.session_state.catalog = fresh
            st.session_state.pricing_checked = now()
            st.rerun()
        except (requests.RequestException, ValueError) as exc:
            st.warning(f'Could not refresh. Existing rates kept. {exc}')
    custom = st.toggle('Use custom rates', help='Overrides session rates. Each request saves the exact rates used.')
    comparison_rates = {name: values.copy() for name, values in st.session_state.catalog[model].items()}
    if custom:
        with st.expander('Edit rates · USD / 1M tokens', expanded=True):
            for processing in ('Standard', 'Batch'):
                st.caption(processing)
                for rate_key, label in RATE_LABELS.items():
                    comparison_rates[processing][rate_key] = st.number_input(
                        f'{processing} · {label}', min_value=0.0,
                        value=float(comparison_rates[processing][rate_key]), step=0.125, format='%.3f',
                        key=f'custom_{model}_{processing}_{rate_key}')
    rates = comparison_rates[mode]
    for rate_key in ('text_input', 'image_input', 'image_output'):
        st.caption(f'{RATE_LABELS[rate_key]}: ${rates[rate_key]:g} / 1M tokens')
    with st.expander('Additional cost adjustment'):
        uplift_pct = st.number_input('Custom uplift (%)', min_value=0.0, max_value=1000.0, value=0.0, step=1.0,
                                     help='Accounting adjustment only. Does not change the API endpoint or account billing.')
        st.caption('Global endpoint. Regional charges, taxes, credits, and account-specific pricing are not detected.')
    st.link_button('Usage & billing ↗', 'https://platform.openai.com/usage')

st.title('🎨 FisoGen')
st.caption('Create images, compare prices, and follow every calculable part of your cost.')
source_links()
create_tab, pricing_tab, jobs_tab, history_tab = st.tabs(
    ['✨ Create / Edit', '💵 Prices & cost planner', '⏳ Batch jobs', '📊 History & costs'])

with create_tab:
    st.session_state.setdefault('batch_draft', [])
    st.session_state.setdefault('batch_draft_revision', 0)
    left, right = st.columns([1, 1], gap='large')
    with left:
        st.subheader('Create with ' + mode)
        if mode == 'Batch':
            st.caption('1. Configure requests here → 2. Add to batch → 3. Review the draft → 4. Submit once.')
        prompt = st.text_area('Prompt' if mode == 'Standard' else 'Batch prompts', height=160, key='prompt',
                              placeholder='Describe your image…',
                              help='Add one prompt at a time, or paste several separated by a line containing only ---. Click Add to batch to capture these prompts and settings.')
        prompts = [item.strip() for item in re.split(r'^\s*---\s*$', prompt, flags=re.M) if item.strip()] if mode == 'Batch' else [prompt.strip()]
        refs = st.file_uploader('Reference images (optional — switches to editing)',
                                type=['png', 'jpg', 'jpeg', 'webp'], accept_multiple_files=True,
                                help='These references are captured when you click Add to batch. Previously added requests keep their own references.')
        if refs:
            st.caption(f'{len(refs)} reference image(s) · input image tokens add to the cost')
            for ref in refs[:4]:
                st.image(ref, caption=ref.name, width=115)
        c1, c2 = st.columns(2)
        with c1:
            quality = st.selectbox('Quality', ['auto', 'low', 'medium', 'high', 'xhigh', 'max'], index=3)
            bg = st.selectbox('Background', ['transparent', 'opaque', 'auto'])
            complexity = st.selectbox('Complexity', ['high', 'medium', 'low'], disabled=not model.startswith('gpt-image-2.6'),
                                       help='Planning effort for 2.6. Exact cost depends on reported token use.')
        with c2:
            size = st.selectbox('Dimensions', ['1024x1024', '1024x1536', '1536x1024', '1536x864', '2048x2048', 'custom'])
            if size == 'custom':
                size = st.text_input('Width x height', '1536x1024')
            fmt = st.selectbox('Output format', ['png', 'webp', 'jpeg'])
            n = st.number_input('Images per request', min_value=1, max_value=10, value=1, step=1)
        valid_size = bool(re.fullmatch(r'[1-9]\d{1,4}x[1-9]\d{1,4}', size))
        if not valid_size:
            st.warning('Enter dimensions as width x height, for example 1536x1024.')
        if bg == 'transparent' and fmt == 'jpeg':
            st.warning('JPEG cannot preserve transparency. Choose PNG or WebP.')
        if len(refs) > 16:
            st.warning('Choose at most 16 reference images.')
        if mode == 'Batch':
            st.info(f'This form: {len(prompts)} request(s) · {len(prompts) * n} image(s). Click Add to batch to include them.')
            st.caption('To paste several prompts at once, put --- on its own line between them. Each will use the current form settings.')
        st.caption('Final cost uses the API’s reported request tokens. Multiple images alone do not enable Batch pricing.')
        valid_form = bool(prompt.strip() and prompts and valid_size and len(refs) <= 16
                          and not (bg == 'transparent' and fmt == 'jpeg'))
        generate = False
        if mode == 'Standard':
            confirm = st.checkbox('I understand this sends a billable API request', value=False)
            generate = st.button('Generate images', type='primary', width='stretch', disabled=not (valid_form and key and confirm))
    with right:
        st.subheader('Batch draft · not sent yet' if mode == 'Batch' else 'Results & request cost')
        params = {'model': model, 'prompt': prompt, 'size': size, 'quality': quality,
                  'background': bg, 'output_format': fmt, 'n': int(n)}
        if model.startswith('gpt-image-2.6'):
            params['complexity'] = complexity
        metadata = {'model': model, 'prompt': prompt, 'size': size, 'quality': quality, 'background': bg, 'format': fmt,
                    'complexity': complexity if model.startswith('gpt-image-2.6') else None,
                    'references': [ref.name for ref in refs], 'requested': int(n), 'mode': mode,
                    'rates': rates.copy(), 'comparison_rates': comparison_rates, 'pricing_source': PRICING_URL,
                    'pricing_checked': st.session_state.pricing_checked,
                    'pricing_basis': 'Custom override' if custom else 'Official published snapshot', 'uplift_pct': uplift_pct}
        if mode == 'Batch':
            with left:
                if st.button('Add to batch', type='primary', width='stretch', disabled=not valid_form):
                    try:
                        additions = draft_entries(params, prompts, refs, metadata)
                        combined = st.session_state.batch_draft + additions
                        serialize_batch([entry['row'] for entry in combined])
                        st.session_state.batch_draft = combined
                        st.session_state.batch_draft_revision += 1
                        st.rerun()
                    except ValueError as exc:
                        st.error(str(exc))
                st.caption('Adding is free and sends nothing to OpenAI. You can change settings before adding the next request.')
            draft = st.session_state.batch_draft
            if draft:
                rows = [entry['row'] for entry in draft]
                payload, endpoint = serialize_batch(rows)
                st.info(f'{len(draft)} request(s) · {sum(row["body"]["n"] for row in rows)} requested image(s) · '
                        f'{len(payload) / 1_000_000:.2f} MB · {rows[0]["body"]["model"]}')
                st.dataframe([{'#': index, 'Prompt': row['body']['prompt'], 'Quality': row['body']['quality'],
                               'Size': row['body']['size'], 'Images': row['body']['n'],
                               'Background': row['body']['background'], 'Format': row['body']['output_format'],
                               'References': len(row['body'].get('images', []))}
                              for index, row in enumerate(rows, 1)], hide_index=True, width='stretch')
                request_labels = {row['custom_id']: f'{index}. {row["body"]["prompt"][:70]}'
                                  for index, row in enumerate(rows, 1)}
                selected = st.selectbox('Review or remove a queued request', list(request_labels),
                                        format_func=lambda request_id, labels=request_labels: labels[request_id])
                selected_entry = next(entry for entry in draft if entry['row']['custom_id'] == selected)
                with st.expander('Selected request · full prompt, settings & rates'):
                    st.write(selected_entry['metadata'])
                if st.button('Remove selected request'):
                    st.session_state.batch_draft = [entry for entry in draft if entry['row']['custom_id'] != selected]
                    st.session_state.batch_draft_revision += 1
                    st.rerun()
                st.download_button('Download draft JSONL', payload, 'image_batch_draft.jsonl', 'application/jsonl',
                                   help='Contains the queued requests and any embedded reference images.')
            else:
                st.info('Your batch is empty. Configure a request on the left and click Add to batch. Repeat as often as needed.')
            st.caption('Only requests listed above will be sent. The form is not automatically included. '
                       'Drafts stay in this browser session; use Download draft JSONL to keep a copy.')
            st.caption('One batch uses one model and one request type (generation or reference-image editing). '
                       'Other settings and reference images can differ per request. Limit: 50,000 requests / 200 MB, '
                       'subject to your account’s queued-token limits. Submitted batches cannot be appended to.')
            confirm = st.checkbox('I understand this sends a billable API request', value=False,
                                  key='confirm_batch_' + str(st.session_state.batch_draft_revision))
            generate = st.button('Submit Batch job', type='primary', width='stretch', disabled=not (draft and key and confirm))
        if generate:
            start = time.monotonic()
            try:
                if mode == 'Batch':
                    request_rows = [entry['row'] for entry in draft]
                    payload, endpoint = serialize_batch(request_rows)
                    with st.spinner('Uploading requests and creating the Batch job…'):
                        job = submit_batch(key, payload, endpoint, request_rows, draft[0]['metadata'], JOBS,
                                           request_metadata={entry['row']['custom_id']: entry['metadata'] for entry in draft})
                    st.session_state.last_batch_id = job['batch_id']
                    st.session_state.batch_draft = []
                    st.session_state.batch_draft_revision += 1
                    st.session_state.batch_notice = f'Batch submitted: {job["batch_id"]}. Open Batch jobs to refresh and collect images.'
                    st.rerun()
                else:
                    endpoint = '/images/edits' if refs else '/images/generations'
                    with st.spinner('Generating… this can take a while'):
                        response = api_request(key, 'POST', endpoint, json=image_body(params, refs), timeout=600)
                    record = make_record(response.json(), metadata, OUT, elapsed_s=round(time.monotonic() - start, 2),
                                         request_id=response.headers.get('x-request-id'))
                    append_record(HISTORY, record)
                    st.session_state.last_record = record
            except (requests.RequestException, ValueError, OSError) as exc:
                st.error(str(exc))
                st.warning('Check Batch jobs and your OpenAI usage dashboard before retrying a timed-out submission; it may have been processed.')
        if st.session_state.get('last_record'):
            show_cost(st.session_state.last_record, prefix='latest', show_images=True)
        elif st.session_state.get('last_batch_id'):
            st.info(f'Latest Batch: {st.session_state.last_batch_id}. Collect results in Batch jobs.')
        else:
            st.info('Reported tokens, itemized costs, and images will appear here. Explore prices and plan costs without an API key.')

with pricing_tab:
    st.subheader('Official prices, in one place')
    st.caption(f'USD per 1 million tokens · checked {st.session_state.pricing_checked} · cached rates are listed for reference; eligibility depends on model and endpoint.')
    st.caption(f'Published rates for {model} · select a model in the sidebar. Custom overrides apply to the planner and new requests below.')
    published = st.session_state.catalog[model]
    st.table([{'Component': label, 'Standard $/1M': f'${published["Standard"][rate_key]:g}',
               'Batch $/1M': f'${published["Batch"][rate_key]:g}'} for rate_key, label in RATE_LABELS.items()])
    search = st.text_input('Find a model or processing mode', placeholder='Try 2.5, sunburst, or Batch')
    price_rows = []
    for name, modes in st.session_state.catalog.items():
        for processing, values in modes.items():
            if search.casefold() in f'{name} {processing}'.casefold():
                price_rows.append({'Model': name, 'Processing': processing,
                                   **{label + ' $/1M': values[k] for k, label in RATE_LABELS.items()}, 'Official source': PRICING_URL})
    st.dataframe(price_rows, hide_index=True, width='stretch',
                 column_config={'Official source': st.column_config.LinkColumn('Official prices', display_text='Open source ↗')})
    st.info('GPT Image 2.5 cached input rates apply only through the Responses API. This app uses the direct Images API. '
            'Cache discounts for 2.6 are estimated only from reported cache details; absent cache data uses uncached rates.')
    st.markdown(f'[Read caching eligibility]({CACHE_URL}) · [Official size & quality calculator]({GUIDE_URL})')
    st.subheader('Plan a workload before spending')
    st.caption(f'{model} · {"custom rates" if custom else "published rates"} · Enter expected tokens from a prior request or the official calculator. These are your assumptions, not measured usage.')
    a, b, c = st.columns(3)
    text_tokens = a.number_input('Text input tokens / request', min_value=0, value=0, step=100)
    image_tokens = b.number_input('Image input tokens / request', min_value=0, value=0, step=100)
    output_tokens = c.number_input('Output tokens / image', min_value=0, value=0, step=100,
                                  help='Use the official calculator or previous usage; dimensions and quality alone do not give exact final usage.')
    a, b, c = st.columns(3)
    planned_images = a.number_input('Planned images / request', min_value=1, max_value=10, value=1)
    planned_requests = b.number_input('Planned requests', min_value=1, max_value=50_000, value=100)
    budget = c.number_input('Budget USD', min_value=0.0, value=10.0, step=1.0)
    planned_usage = {'input_tokens_details': {'text_tokens': text_tokens, 'image_tokens': image_tokens},
                     'input_tokens': text_tokens + image_tokens, 'output_tokens': output_tokens * planned_images}
    if text_tokens + image_tokens + output_tokens:
        plan = {processing: estimate(planned_usage, values, uplift_pct=uplift_pct) for processing, values in comparison_rates.items()}
        standard_total = plan['Standard']['total'] * planned_requests
        batch_total = plan['Batch']['total'] * planned_requests
        a, b, c = st.columns(3)
        a.metric('Standard workload', money(standard_total))
        b.metric('Batch workload', money(batch_total))
        c.metric('Potential savings', money(standard_total - batch_total))
        st.dataframe([{'Processing': processing, 'Text input USD / request': result['text'],
                       'Image input USD / request': result['image'], 'Output USD / request': result['output'],
                       'Adjustment USD / request': result['uplift'], 'Total USD / request': result['total'],
                       'Average USD / image': result['total'] / planned_images,
                       'Requests within budget': int(budget // result['total']) if result['total'] else None}
                      for processing, result in plan.items()], hide_index=True, width='stretch')
        st.caption(f'{planned_requests:,} requests × {planned_images} images = {planned_requests * planned_images:,} images. '
                   f'Custom adjustment: {uplift_pct:g}%. Only entered components are included; omitted tokens can understate the plan.')
    else:
        st.info('Enter expected token quantities to compare Standard and Batch. No API request is made by this planner.')
    with st.expander('How every cost is calculated', expanded=True):
        st.markdown('''- **Text input:** prompt tokens × text rate / 1,000,000.
- **Image input:** reference image tokens × image input rate / 1,000,000.
- **Image output:** all output tokens reported for the request × output rate / 1,000,000.
- **Cached inputs:** charged separately when eligible and reported; subtracted from uncached inputs.
- **Request total:** sum of components, plus any custom adjustment. Missing components keep the total unavailable.
- **Per-image cost:** request total / images returned. Reported output tokens cover the whole request and are never multiplied by the requested image count.
- **Batch:** actual queued processing with separate rates and a 24-hour completion window. Completed requests in expired or cancelled batches can still incur costs.
- **Quality, dimensions, references, and complexity:** can affect token use. Token rates alone cannot predict an exact final cost.
- **Billing:** taxes, credits, regional uplifts, negotiated prices, and unreported usage are not automatically detected. The account billing dashboard is authoritative.''')

with jobs_tab:
    st.subheader('Queued Batch jobs')
    st.caption('Jobs survive restarts. Refresh retrieves status and imports available results once, using custom_id to match prompts.')
    jobs = load_jobs(JOBS)
    if not jobs:
        st.info('Choose Batch processing in the sidebar and submit prompts from Create / Edit.')
    for job in jobs:
        label = job.get('batch_id', job['local_id'])
        with st.expander(f'{label} · {job["status"]} · {len(job["requests"])} request(s)', expanded=job['status'] not in TERMINAL):
            st.caption(f'Created {job["time"]} · {job["metadata"]["model"]} · saved Batch rates')
            if job.get('error'):
                st.warning(job['error'])
            if job.get('batch_id'):
                batch = job.get('batch') or {}
                st.write('Request progress', batch.get('request_counts', {}))
                if batch.get('errors'):
                    st.error(str(batch['errors']))
                if st.button('Refresh status & collect results', key='refresh_' + job['local_id'], disabled=not key):
                    try:
                        with st.spinner('Checking Batch and importing available results…'):
                            imported = refresh_batch(key, job, JOBS, OUT, HISTORY)
                        st.session_state.batch_notice = f'{label}: {job["status"]}. Imported {imported} result(s). Open History & costs to view images and charges.'
                        st.rerun()
                    except (requests.RequestException, ValueError, OSError) as exc:
                        st.error(str(exc))
                st.json({name: batch.get(name) for name in ('status', 'request_counts', 'created_at', 'in_progress_at',
                          'completed_at', 'expires_at', 'output_file_id', 'error_file_id')})
            elif job['status'] in ('submission_unknown', 'submitting'):
                st.warning('Submission outcome is unknown. Check your OpenAI Batch dashboard before creating another job.')
                st.link_button('Open Batch dashboard ↗', 'https://platform.openai.com/batches')
                recovered = st.text_input('Recover Batch ID from the dashboard', key='recover_' + job['local_id'])
                if st.button('Attach Batch ID', key='attach_' + job['local_id'], disabled=not key):
                    if not re.fullmatch(r'batch_[A-Za-z0-9_-]+', recovered):
                        st.error('Enter a valid batch_… ID.')
                    else:
                        try:
                            candidate = api_request(key, 'GET', '/batches/' + recovered, timeout=60).json()
                            if candidate.get('input_file_id') != job.get('input_file_id'):
                                st.error('This Batch uses a different input file and cannot be attached to this local job.')
                            else:
                                job.update(batch_id=recovered, status=candidate['status'], batch=candidate)
                                save_job(JOBS, job)
                                st.rerun()
                        except (requests.RequestException, ValueError, OSError) as exc:
                            st.error(str(exc))
            st.caption('Job rate snapshot · individual requests also retain their own saved settings and rates')
            st.json(job['metadata']['rates'])
    if st.session_state.get('batch_notice'):
        st.success(st.session_state.batch_notice)
    st.link_button('Open OpenAI Batch dashboard ↗', 'https://platform.openai.com/batches')
    st.caption('Collect results within 30 days; OpenAI Batch output and error files expire by default. '
               'Failed or expired requests with no reported usage have unknown costs here.')

with history_tab:
    rows = read_history(HISTORY)
    if not rows:
        st.info('No requests recorded yet. History stays on this computer.')
    else:
        a, b, c = st.columns(3)
        model_filter = a.multiselect('Filter models', sorted({r['model'] for r in rows}))
        mode_filter = b.multiselect('Filter processing', ['Standard', 'Batch'])
        quality_filter = c.multiselect('Filter quality', sorted({r['quality'] for r in rows}))
        filtered = [r for r in rows if (not model_filter or r['model'] in model_filter)
                    and (not mode_filter or r.get('mode', 'Standard') in mode_filter)
                    and (not quality_filter or r['quality'] in quality_filter)]
        known = [r for r in filtered if r.get('cost') and r['cost'].get('total') is not None]
        total = sum(r['cost']['total'] for r in known)
        image_count = sum(r.get('returned_images', len(r.get('files', []))) for r in known)
        a, b, c, d = st.columns(4)
        a.metric('Requests shown', len(filtered))
        b.metric('Images saved', sum(len(r.get('files', [])) for r in filtered))
        c.metric('Complete estimated spend', money(total))
        d.metric('Average / priced image', money(total / image_count) if image_count else 'Unavailable')
        if len(known) < len(filtered):
            st.warning(f'{len(filtered) - len(known)} request(s) have incomplete costs and are excluded from spend and average. Unknown is not zero.')
        data = []
        for record in filtered:
            usage, cost = record.get('usage') or {}, record.get('cost') or {}
            details = usage.get('input_tokens_details') or {}
            data.append({'Time (UTC)': record['time'], 'Model': record['model'], 'Processing': record.get('mode', 'Standard'),
                         'Quality': record['quality'], 'Size': record['size'], 'References': len(record.get('references', [])),
                         'Images saved': len(record.get('files', [])), 'Images requested': record['requested'],
                         'Text tokens': details.get('text_tokens'), 'Image input tokens': details.get('image_tokens'),
                         'Output tokens': usage.get('output_tokens'), 'Total tokens': usage.get('total_tokens'),
                         'Text cost USD': cost.get('text'), 'Image input cost USD': cost.get('image'), 'Output cost USD': cost.get('output'),
                         'Adjustment USD': cost.get('uplift'), 'Total USD': cost.get('total'), 'Cost complete': cost.get('total') is not None,
                         'Pricing basis': record.get('pricing_basis', 'Legacy snapshot'), 'Prices checked': record.get('pricing_checked'),
                         'Seconds': record.get('elapsed_s'), 'Batch ID': record.get('batch_id'),
                         'Error': json.dumps(record.get('error')) if record.get('error') else None,
                         **{'Rate ' + k + ' USD/1M': v for k, v in record.get('rates', {}).items()}})
        if data:
            st.dataframe(data, width='stretch', hide_index=True,
                         column_config={name: st.column_config.NumberColumn(format='$%.6f') for name in
                                        ('Text cost USD', 'Image input cost USD', 'Output cost USD', 'Adjustment USD', 'Total USD')})
            st.download_button('Export filtered history CSV', pd.DataFrame(data).to_csv(index=False).encode('utf-8'), 'image_cost_history.csv', 'text/csv')
            st.download_button('Export filtered details JSONL',
                               ('\n'.join(json.dumps(row, ensure_ascii=False) for row in filtered) + '\n').encode('utf-8'),
                               'image_cost_history.jsonl', 'application/jsonl')
            st.subheader('Inspect a request')
            selected = st.selectbox('Request', range(len(filtered)),
                                    format_func=lambda index: f'{filtered[index]["time"][:19]} · {filtered[index].get("mode", "Standard")} · {filtered[index]["model"]} · {filtered[index]["prompt"][:60]}')
            show_cost(filtered[selected], prefix='history_' + str(selected), show_images=True)
        else:
            st.info('No requests match these filters.')
        st.caption('History includes prompts, reference filenames, rate snapshots, and API usage. API keys are never stored. '
                   'Historical costs remain as recorded; current sidebar prices do not reprice them.')
