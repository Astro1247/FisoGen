"""Run with python -m unittest -v. All billable API calls are mocked."""

import base64
import io
import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import requests
from PIL import Image
from streamlit.testing.v1 import AppTest

from pricing import (BATCH, MODELS, STANDARD, default_catalog, estimate,
                     parse_official_prices)
from studio_api import (build_batch, draft_entries, image_body, load_jobs, make_record,
                        read_history, refresh_batch, serialize_batch, submit_batch)


def usage(text=1000, image=2000, output=3000):
    return {'input_tokens': text + image, 'input_tokens_details': {'text_tokens': text, 'image_tokens': image},
            'output_tokens': output, 'total_tokens': text + image + output}


def png():
    buffer = io.BytesIO()
    Image.new('RGB', (2, 2), 'blue').save(buffer, format='PNG')
    return buffer.getvalue()


def response(body=None, text=None):
    result = requests.Response()
    result.status_code = 200
    result._content = (text if text is not None else json.dumps(body)).encode('utf-8')
    return result


def metadata():
    return {'model': MODELS[0], 'format': 'png', 'quality': 'high', 'size': '1024x1024',
            'prompt': 'test', 'requested': 2, 'rates': BATCH.copy(), 'mode': 'Batch',
            'comparison_rates': default_catalog()[MODELS[0]], 'uplift_pct': 0}


class PricingTests(unittest.TestCase):
    def test_standard_batch_and_adjustment(self):
        standard = estimate(usage(), STANDARD)
        batch = estimate(usage(), BATCH, uplift_pct=10)
        self.assertAlmostEqual(standard['total'], 0.111)
        self.assertAlmostEqual(batch['subtotal'], 0.0555)
        self.assertAlmostEqual(batch['total'], 0.06105)
        self.assertEqual(standard['lines'][4]['tokens'], 3000)

    def test_incomplete_usage_preserves_known_components(self):
        result = estimate({'input_tokens': 400, 'output_tokens': 1000}, STANDARD)
        self.assertIsNone(result['total'])
        self.assertAlmostEqual(result['known_total'], 0.03)
        result = estimate({'input_tokens_details': {'text_tokens': 1000, 'image_tokens': 0}}, STANDARD)
        self.assertIsNone(result['total'])
        self.assertAlmostEqual(result['known_total'], 0.005)
        self.assertIsNone(estimate(None, STANDARD))

    def test_zero_is_not_missing(self):
        self.assertEqual(estimate(usage(0, 0, 0), STANDARD)['total'], 0)
        self.assertEqual(estimate({'input_tokens': 0, 'output_tokens': 0}, STANDARD)['total'], 0)

    def test_cache_split_and_eligibility(self):
        raw = usage()
        raw['input_tokens_details'].update(cached_tokens=1200, cached_tokens_details={'text_tokens': 200, 'image_tokens': 1000})
        cached = estimate(raw, STANDARD, cache_eligible=True)
        self.assertAlmostEqual(cached['total'], 0.10425)
        self.assertAlmostEqual(estimate(raw, STANDARD, cache_eligible=False)['total'], 0.111)
        raw['input_tokens_details'].pop('cached_tokens_details')
        self.assertIsNone(estimate(raw, STANDARD, cache_eligible=True)['total'])

    def test_invalid_reported_counts_are_not_silently_priced(self):
        raw = usage()
        raw['input_tokens'] += 1
        self.assertIsNone(estimate(raw, STANDARD)['total'])
        raw = usage()
        raw['output_tokens'] = -1
        self.assertIsNone(estimate(raw, STANDARD)['total'])
        raw = usage()
        raw['input_tokens_details']['cached_tokens_details'] = {'text_tokens': 2000, 'image_tokens': 0}
        self.assertIsNone(estimate(raw, STANDARD, cache_eligible=True)['total'])

    def test_official_parser_does_not_confuse_modalities_or_processing(self):
        chunks = ['Images']
        for mode, rates in [('Standard', STANDARD), ('Batch', BATCH)]:
            chunks.append(mode)
            for model in MODELS:
                chunks.append(f'| {model} | Image | ${rates["image_input"]} | ${rates["image_output"]} | ${rates["cached_image_input"]} |')
                chunks.append(f'| {model} | Text | ${rates["text_input"]} | - | ${rates["cached_text_input"]} |')
        chunks.append('Audio')
        self.assertEqual(parse_official_prices('\n'.join(chunks)), default_catalog())
        with self.assertRaises(ValueError):
            parse_official_prices('\n'.join(chunks[:-3]))
        with self.assertRaises(ValueError):
            parse_official_prices('The page has changed')


class BatchTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.jobs = self.root / 'batches'
        self.out = self.root / 'outputs'
        self.out.mkdir()
        self.history = self.root / 'history.jsonl'
        self.params = {'model': MODELS[0], 'n': 2, 'prompt': 'test', 'output_format': 'png'}

    def test_batch_payload_and_edit_json(self):
        ref = io.BytesIO(png())
        ref.name = 'reference.png'
        payload, endpoint, rows = build_batch(self.params, ['one', 'two'], [ref])
        self.assertEqual(endpoint, '/v1/images/edits')
        self.assertEqual(len(payload.splitlines()), 2)
        self.assertNotEqual(rows[0]['custom_id'], rows[1]['custom_id'])
        self.assertEqual(rows[1]['body']['prompt'], 'two')
        self.assertTrue(rows[0]['body']['images'][0]['image_url'].startswith('data:image/png;base64,'))
        self.assertNotIn('images', image_body(self.params, []))

    @patch('studio_api.api_request')
    def test_submit_and_import_out_of_order_partial_failure_once(self, api):
        payload, endpoint, rows = build_batch(self.params, ['first', 'second'], [])
        api.side_effect = [response({'id': 'file-input'}), response({'id': 'batch_test', 'status': 'validating'})]
        job = submit_batch('secret-test-key', payload, endpoint, rows, metadata(), self.jobs)
        self.assertEqual(api.call_args_list[0].kwargs['data']['purpose'], 'batch')
        self.assertEqual(api.call_args_list[1].kwargs['json']['completion_window'], '24h')
        self.assertNotIn('secret-test-key', next(self.jobs.glob('*.json')).read_text())
        result = {'custom_id': 'image-00002', 'response': {'status_code': 200, 'request_id': 'req_test',
                  'body': {'usage': usage(), 'data': [{'b64_json': base64.b64encode(png()).decode()}]}}, 'error': None}
        failed = {'custom_id': 'image-00001', 'response': None,
                  'error': {'code': 'batch_expired', 'message': 'expired'}}
        batch = {'id': 'batch_test', 'status': 'expired', 'output_file_id': 'file-output', 'error_file_id': 'file-error'}
        api.side_effect = [response(batch), response(text=json.dumps(result)), response(text=json.dumps(failed))]
        self.assertEqual(refresh_batch('key', job, self.jobs, self.out, self.history), 2)
        records = read_history(self.history)
        success = next(record for record in records if not record['error'])
        self.assertEqual(success['prompt'], 'second')
        self.assertAlmostEqual(success['cost']['total'], 0.0555)
        self.assertEqual(len(success['files']), 1)
        self.assertIsNone(next(record for record in records if record['error'])['cost'])
        api.side_effect = [response(batch)]
        self.assertEqual(refresh_batch('key', load_jobs(self.jobs)[0], self.jobs, self.out, self.history), 0)
        self.assertEqual(len(read_history(self.history)), 2)
        self.assertEqual(len(list(self.out.iterdir())), 1)

    @patch('studio_api.api_request')
    def test_timeout_keeps_recoverable_input_file_and_rate_snapshot(self, api):
        payload, endpoint, rows = build_batch(self.params, ['one'], [])
        api.side_effect = [response({'id': 'file-input'}), requests.Timeout('uncertain')]
        with self.assertRaises(requests.Timeout):
            submit_batch('key', payload, endpoint, rows, metadata(), self.jobs)
        job = load_jobs(self.jobs)[0]
        self.assertEqual(job['status'], 'submission_unknown')
        self.assertEqual(job['input_file_id'], 'file-input')
        self.assertEqual(job['metadata']['rates'], BATCH)

    def test_multiple_images_do_not_multiply_reported_usage(self):
        body = {'usage': usage(), 'data': [{'b64_json': base64.b64encode(png()).decode()}] * 2}
        record = make_record(body, metadata(), self.out)
        self.assertEqual(record['returned_images'], 2)
        self.assertAlmostEqual(record['cost']['total'], 0.0555)

    def test_draft_rejects_mixed_models_endpoints_and_limits(self):
        first = draft_entries(self.params, ['first'], [], metadata())[0]
        second = draft_entries({**self.params, 'model': MODELS[1]}, ['second'], [], metadata())[0]
        with self.assertRaisesRegex(ValueError, 'same model'):
            serialize_batch([first['row'], second['row']])
        second['row']['body']['model'] = MODELS[0]
        second['row']['url'] = '/v1/images/edits'
        with self.assertRaisesRegex(ValueError, 'separate batches'):
            serialize_batch([first['row'], second['row']])
        with patch('studio_api.MAX_BATCH_BYTES', 10), self.assertRaisesRegex(ValueError, '200 MB'):
            serialize_batch([first['row']])

    @patch('studio_api.api_request')
    def test_import_keeps_per_request_settings_and_rates(self, api):
        first = draft_entries({**self.params, 'quality': 'high'}, ['first'], [], metadata())[0]
        second_meta = {**metadata(), 'format': 'webp', 'quality': 'low', 'requested': 3,
                       'rates': {**BATCH, 'image_output': 20.0}}
        second = draft_entries({**self.params, 'quality': 'low', 'n': 3, 'output_format': 'webp'},
                               ['second'], [], second_meta)[0]
        entries = [first, second]
        rows = [entry['row'] for entry in entries]
        payload, endpoint = serialize_batch(rows)
        api.side_effect = [response({'id': 'file-input'}), response({'id': 'batch_mixed', 'status': 'validating'})]
        job = submit_batch('key', payload, endpoint, rows, first['metadata'], self.jobs,
                           request_metadata={entry['row']['custom_id']: entry['metadata'] for entry in entries})
        results = [{'custom_id': entry['row']['custom_id'], 'response': {'status_code': 200,
                    'body': {'usage': usage(), 'data': []}}} for entry in reversed(entries)]
        api.side_effect = [response({'status': 'completed', 'output_file_id': 'file-output'}),
                           response(text='\n'.join(json.dumps(row) for row in results))]
        refresh_batch('key', load_jobs(self.jobs)[0], self.jobs, self.out, self.history)
        imported = {record['prompt']: record for record in read_history(self.history)}
        self.assertEqual(imported['second']['format'], 'webp')
        self.assertEqual(imported['second']['quality'], 'low')
        self.assertEqual(imported['second']['requested'], 3)
        self.assertAlmostEqual(imported['first']['cost']['total'], 0.0555)
        self.assertAlmostEqual(imported['second']['cost']['total'], 0.0705)


class AppTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        source = Path('app.py').read_text(encoding='utf-8').replace('ROOT = Path(__file__).parent', f'ROOT = Path({str(self.root)!r})')
        with patch.dict(os.environ, {'OPENAI_API_KEY': ''}):
            self.app = AppTest.from_string(source, default_timeout=20).run()
        self.assertFalse(self.app.exception)

    def by_label(self, kind, label):
        return next(element for element in getattr(self.app, kind) if element.label == label)

    def test_pricing_planner_custom_rates_and_search(self):
        self.assertTrue(self.by_label('button', 'Generate images').disabled)
        self.by_label('number_input', 'Text input tokens / request').set_value(1000)
        self.by_label('number_input', 'Output tokens / image').set_value(3000)
        self.app.run()
        self.assertFalse(self.app.exception)
        self.assertEqual(next(m.value for m in self.app.metric if m.label == 'Standard workload'), '$9.500000')
        self.assertEqual(next(m.value for m in self.app.metric if m.label == 'Batch workload'), '$4.750000')
        self.by_label('text_input', 'Find a model or processing mode').set_value('2.5')
        self.app.run()
        self.assertEqual(len(self.app.dataframe[0].value), 4)
        self.by_label('toggle', 'Use custom rates').set_value(True)
        self.app.run()
        self.by_label('number_input', 'Standard · Image output').set_value(60.0)
        self.app.run()
        self.assertEqual(next(m.value for m in self.app.metric if m.label == 'Standard workload'), '$18.500000')
        self.assertFalse(self.app.exception)

    @patch('studio_api.api_request')
    def test_standard_generation_and_history_without_live_api(self, api):
        api.return_value = response({'usage': usage(), 'data': [{'b64_json': base64.b64encode(png()).decode()}]})
        self.by_label('text_input', 'OpenAI API key').set_value('test-key')
        self.app.text_area[0].set_value('A blue square')
        self.app.checkbox[0].check()
        self.app.run()
        self.by_label('button', 'Generate images').click().run()
        self.assertFalse(self.app.exception)
        self.assertEqual(api.call_count, 1)
        records = read_history(self.root / 'history.jsonl')
        self.assertEqual(len(records), 1)
        self.assertAlmostEqual(records[0]['cost']['total'], 0.111)
        self.assertTrue(any(m.label == 'Complete estimated spend' for m in self.app.metric))
        self.by_label('multiselect', 'Filter processing').set_value(['Batch']).run()
        self.assertFalse(self.app.exception)

    @patch('studio_api.api_request')
    def test_batch_submission_ui(self, api):
        api.side_effect = [response({'id': 'file-input'}), response({'id': 'batch_ui', 'status': 'validating'})]
        self.by_label('radio', 'Processing').set_value('Batch').run()
        self.app.text_area[0].set_value('A blue square').run()
        self.by_label('button', 'Add to batch').click().run()
        self.assertEqual(api.call_count, 0)
        self.assertTrue(self.by_label('button', 'Submit Batch job').disabled)
        self.app.text_area[0].set_value('A red circle')
        self.by_label('selectbox', 'Quality').set_value('low')
        self.by_label('number_input', 'Images per request').set_value(3)
        self.app.run()
        self.by_label('button', 'Add to batch').click().run()
        self.assertEqual(api.call_count, 0)
        self.by_label('text_input', 'OpenAI API key').set_value('test-key')
        self.app.checkbox[0].check()
        self.app.run()
        self.by_label('button', 'Submit Batch job').click().run()
        self.assertFalse(self.app.exception)
        self.assertEqual(api.call_count, 2)
        job = load_jobs(self.root / 'batches')[0]
        self.assertEqual(len(job['requests']), 2)
        individual = list(job['request_metadata'].values())
        self.assertEqual([item['requested'] for item in individual], [1, 3])
        self.assertEqual([item['quality'] for item in individual], ['high', 'low'])
        self.assertEqual(self.app.session_state['batch_draft'], [])
        self.assertFalse((self.root / 'history.jsonl').exists())

    @patch('studio_api.api_request')
    def test_batch_bulk_add_remove_and_reject_model_change(self, api):
        self.by_label('radio', 'Processing').set_value('Batch').run()
        self.app.text_area[0].set_value('One\n---\nTwo').run()
        self.by_label('button', 'Add to batch').click().run()
        self.assertEqual(len(self.app.session_state['batch_draft']), 2)
        self.by_label('selectbox', 'Model').set_value(MODELS[1]).run()
        self.by_label('button', 'Add to batch').click().run()
        self.assertFalse(self.app.exception)
        self.assertTrue(any('same model' in error.value for error in self.app.error))
        self.assertEqual(len(self.app.session_state['batch_draft']), 2)
        self.by_label('button', 'Remove selected request').click().run()
        self.assertEqual(len(self.app.session_state['batch_draft']), 1)
        self.by_label('button', 'Remove selected request').click().run()
        self.assertEqual(len(self.app.session_state['batch_draft']), 0)
        self.assertFalse(self.app.exception)
        self.assertEqual(api.call_count, 0)


if __name__ == '__main__':
    unittest.main()
