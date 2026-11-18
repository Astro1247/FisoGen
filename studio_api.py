"""Images and Batch transport, local job persistence, and result import."""

import base64
import io
import json
import re
import uuid
from datetime import datetime, timezone

import requests

from pricing import PRICING_URL, estimate

API = 'https://api.openai.com/v1'
TERMINAL = {'completed', 'failed', 'expired', 'cancelled'}
MAX_BATCH_REQUESTS = 50_000
MAX_BATCH_BYTES = 200_000_000


def now():
    return datetime.now(timezone.utc).isoformat()


def read_history(path):
    rows = []
    if path.exists():
        for line in path.read_text(encoding='utf-8').splitlines():
            try:
                rows.append(json.loads(line))
            except json.JSONDecodeError:
                continue
    return list(reversed(rows))


def append_record(path, record):
    with path.open('a', encoding='utf-8') as handle:
        handle.write(json.dumps(record, ensure_ascii=False) + '\n')


def api_request(key, method, path, **kwargs):
    response = requests.request(method, API + path,
                                headers={'Authorization': f'Bearer {key}'}, **kwargs)
    if not response.ok:
        try:
            error = response.json().get('error', {})
            message = error.get('message', str(error)) if isinstance(error, dict) else str(error)
        except ValueError:
            message = response.text[:500]
        raise requests.HTTPError(f'OpenAI HTTP {response.status_code}: {message}', response=response)
    return response


def image_body(params, references):
    body = params.copy()
    if references:
        if len(references) > 16:
            raise ValueError('The Images API supports at most 16 reference images.')
        body['images'] = []
        for ref in references:
            mime = {'png': 'image/png', 'jpg': 'image/jpeg', 'jpeg': 'image/jpeg', 'webp': 'image/webp'}[ref.name.rsplit('.', 1)[-1].lower()]
            url = 'data:' + mime + ';base64,' + base64.b64encode(ref.getvalue()).decode('ascii')
            if len(url) > 20_971_520:
                raise ValueError(f'{ref.name} exceeds the JSON image-reference size limit.')
            body['images'].append({'image_url': url})
    return body


def build_batch(params, prompts, references):
    body = image_body(params, references)
    endpoint = '/v1/images/edits' if references else '/v1/images/generations'
    requests_ = [{'custom_id': f'image-{index:05d}', 'method': 'POST', 'url': endpoint,
                  'body': {**body, 'prompt': prompt}} for index, prompt in enumerate(prompts, 1)]
    payload, endpoint = serialize_batch(requests_)
    return payload, endpoint, requests_


def serialize_batch(requests_):
    """Validate a draft with per-request settings before any network operation."""
    if not requests_ or len(requests_) > MAX_BATCH_REQUESTS:
        raise ValueError('A batch must contain between 1 and 50,000 requests.')
    if len({row['body']['model'] for row in requests_}) != 1:
        raise ValueError('One batch must use the same model. Submit the current draft before switching models.')
    endpoints = {row['url'] for row in requests_}
    if len(endpoints) != 1:
        raise ValueError('Generations and reference-image edits need separate batches. Keep the same request type in this draft.')
    if len({row['custom_id'] for row in requests_}) != len(requests_):
        raise ValueError('Every request must have a unique ID.')
    payload = io.BytesIO()
    for row in requests_:
        line = (json.dumps(row, ensure_ascii=False) + '\n').encode('utf-8')
        if payload.tell() + len(line) > MAX_BATCH_BYTES:
            raise ValueError('Batch input exceeds 200 MB. Reduce the number of prompts or reference sizes.')
        payload.write(line)
    return payload.getvalue(), next(iter(endpoints))


def draft_entries(params, prompts, references, metadata):
    """Capture independent request settings; adding a draft never calls the API."""
    body = image_body(params, references)
    endpoint = '/v1/images/edits' if references else '/v1/images/generations'
    return [{'row': {'custom_id': 'image-' + uuid.uuid4().hex, 'method': 'POST', 'url': endpoint,
                     'body': {**body, 'prompt': prompt}},
             'metadata': {**metadata, 'prompt': prompt}} for prompt in prompts]


def save_job(directory, job):
    directory.mkdir(exist_ok=True)
    path = directory / (job['local_id'] + '.json')
    temporary = path.with_suffix('.tmp')
    temporary.write_text(json.dumps(job, ensure_ascii=False, indent=2), encoding='utf-8')
    temporary.replace(path)


def load_jobs(directory):
    jobs = []
    if directory.exists():
        for path in directory.glob('*.json'):
            try:
                jobs.append(json.loads(path.read_text(encoding='utf-8')))
            except (ValueError, OSError):
                continue
    return sorted(jobs, key=lambda job: job['time'], reverse=True)


def submit_batch(key, payload, endpoint, request_rows, metadata, directory, *, request_metadata=None):
    job = {'local_id': uuid.uuid4().hex, 'time': now(), 'status': 'uploading',
           'endpoint': endpoint, 'metadata': metadata,
           'requests': {row['custom_id']: row['body']['prompt'] for row in request_rows}}
    if request_metadata is not None:
        job['request_metadata'] = request_metadata
    save_job(directory, job)
    try:
        uploaded = api_request(key, 'POST', '/files', data={'purpose': 'batch'},
                               files={'file': ('images.jsonl', payload, 'application/jsonl')}, timeout=120).json()
        job.update(input_file_id=uploaded['id'], status='submitting')
        save_job(directory, job)
        batch = api_request(key, 'POST', '/batches', json={
            'input_file_id': uploaded['id'], 'endpoint': endpoint, 'completion_window': '24h',
            'metadata': {'studio_job': job['local_id']}}, timeout=120).json()
        job.update(batch_id=batch['id'], status=batch['status'], batch=batch)
    except Exception as error:
        job['status'] = 'submission_unknown' if job['status'] == 'submitting' and not isinstance(error, requests.HTTPError) else 'submission_failed'
        job['error'] = str(error)
        save_job(directory, job)
        raise
    save_job(directory, job)
    return job


def make_record(body, metadata, output_dir, *, record_id=None, error=None, **extra):
    record_id = record_id or uuid.uuid4().hex
    files, output_errors = [], []
    for index, item in enumerate(body.get('data') or [], 1):
        encoded = item.get('b64_json')
        if not encoded:
            output_errors.append(f'Output {index} did not contain a base64 image.')
            continue
        try:
            raw = base64.b64decode(encoded, validate=True)
            filename = f'{record_id}_{index}.{metadata["format"]}'
            (output_dir / filename).write_bytes(raw)
            files.append(filename)
        except (ValueError, OSError) as exc:
            output_errors.append(f'Output {index} could not be saved: {exc}')
    usage = body.get('usage')
    eligible = metadata['model'].startswith('gpt-image-2.6')
    cost = estimate(usage, metadata['rates'], cache_eligible=eligible,
                    uplift_pct=metadata.get('uplift_pct', 0))
    return {**metadata, 'record_id': record_id, 'time': now(), 'files': files,
            'returned_images': len(body.get('data') or []), 'usage': usage, 'cost': cost,
            'response_id': body.get('id'), 'error': error, 'output_errors': output_errors,
            'pricing_source': metadata.get('pricing_source', PRICING_URL), **extra}


def refresh_batch(key, job, directory, output_dir, history_path):
    if not re.fullmatch(r'batch_[A-Za-z0-9_-]+', job['batch_id']):
        raise ValueError('Invalid Batch ID.')
    batch = api_request(key, 'GET', '/batches/' + job['batch_id'], timeout=60).json()
    job.update(status=batch['status'], batch=batch, checked_at=now())
    save_job(directory, job)
    seen = {record.get('record_id') for record in read_history(history_path)}
    imported = 0
    for file_key in ('output_file_id', 'error_file_id'):
        file_id = batch.get(file_key)
        if not file_id or file_id in job.get('imported_files', []):
            continue
        if not re.fullmatch(r'file-[A-Za-z0-9_-]+|file_[A-Za-z0-9_-]+', file_id):
            raise ValueError('Invalid output file ID.')
        content = api_request(key, 'GET', '/files/' + file_id + '/content', timeout=120).text
        for line in content.splitlines():
            if not line.strip():
                continue
            result = json.loads(line)
            custom_id = result['custom_id']
            if custom_id not in job['requests']:
                raise ValueError(f'Unknown custom_id in Batch results: {custom_id}')
            record_id = uuid.uuid5(uuid.NAMESPACE_URL, job['batch_id'] + '/' + custom_id).hex
            if record_id in seen:
                continue
            response = result.get('response') or {}
            body = response.get('body') or {}
            error = result.get('error') or body.get('error')
            if response.get('status_code', 200) != 200 and not error:
                error = f'HTTP {response["status_code"]}'
            metadata = {**job.get('request_metadata', {}).get(custom_id, job['metadata']),
                        'prompt': job['requests'][custom_id]}
            record = make_record(body, metadata, output_dir, record_id=record_id, error=error,
                                 batch_id=job['batch_id'], custom_id=custom_id,
                                 request_id=response.get('request_id'), elapsed_s=None)
            append_record(history_path, record)
            seen.add(record_id)
            imported += 1
        job.setdefault('imported_files', []).append(file_id)
        save_job(directory, job)
    return imported
