# Copyright (C) 2026 Toit contributors.
import argparse
import copy
import json
import unittest
from unittest.mock import patch

import ollama_provider as provider


class ProviderTests(unittest.TestCase):
    def setUp(self):
        self.request = {'schema_version': 1, 'task': 'suggest_candidates',
            'targets': [{'left_context': 'The parcel arrived by ',
                         'right_context': ' before noon.', 'surface_kind': 'canonical'}]}
        self.incremental = dict(self.request, schema_version=2, task='extend_prefixes',
            surface_kind='canonical', prefixes=[{'id': 'p', 'text': 'the '}],
            limits={'max_extensions': 2, 'max_text_bytes': 20})

    def test_context_and_feedback_preserved(self):
        self.incremental['prefixes'][0]['observations'] = [{'remaining_pixels': [10, 20]}]
        context, schema, kind = provider.prepare(self.incremental, 16)
        self.assertIn('arrived by [REDACTED] before', context['occurrences'][0]['text_with_gap'])
        self.assertEqual(context['occurrences'][0]['right_context'], ' before noon.')
        self.assertEqual(context['prefixes'], self.incremental['prefixes'])
        self.assertEqual(schema['properties']['extensions']['maxItems'], 2)

    def test_candidate_dedup_and_no_probability(self):
        result = provider.normalize(self.request, {'texts': ['mail', 'courier', 'mail']}, 4, 'canonical', 'local:test')
        self.assertEqual(len(result['candidates']), 2)
        self.assertIsNone(result['candidates'][0]['context_score'])
        for texts in [['a\nb'], ['x'*513], [''], [123]]:
            with self.assertRaises(ValueError):
                provider.normalize(self.request, {'texts': texts}, 4, 'canonical', 'x')

    def test_only_exact_context_wrappers_are_removed(self):
        sentence = 'The parcel arrived by courier before noon.'
        self.assertEqual(provider.extract_replacement(sentence, self.request['targets']), 'courier')
        different = 'The parcel arrived by courier tomorrow.'
        self.assertEqual(provider.extract_replacement(different, self.request['targets']), different)

    def test_extension_spacing_and_non_exhaustiveness(self):
        extension = {'parent_id': 'p', 'append': 'parcel', 'complete': True}
        result = provider.normalize(self.incremental, {'extensions': [extension]}, 2, 'canonical', 'x')
        self.assertEqual(result['extensions'], [extension])
        self.assertFalse(result['exhaustive_for_prefixes'])
        for changes in [{'parent_id': 'invented'}, {'append': '', 'complete': False},
                        {'complete': 'true'}, {'append': 'x'*30}, {'append': '\n'}]:
            invalid = dict(extension, **changes)
            with self.assertRaises(ValueError):
                provider.normalize(self.incremental, {'extensions': [invalid]}, 2, 'canonical', 'x')

    def test_local_endpoint_only(self):
        for url in ['https://example.com', 'http://192.168.1.1:11434',
                    'http://127.0.0.1:11434/path', 'http://user:password@localhost:11434']:
            with self.assertRaises(ValueError):
                provider.LocalOllama(url, 1)
        provider.LocalOllama('http://127.0.0.1:11434', 1)
        provider.LocalOllama('http://[::1]:11434', 1)
        with self.assertRaises(ValueError):
            provider.NoRedirect().redirect_request(None, None, 302, None, None, 'https://example.com')

    def test_expansion_uses_context_ids_and_retains_casing(self):
        request = dict(schema_version=1, task='propose_context_candidates', surface_kind='canonical',
                       contexts={'t': dict(left='met', right='at the station',
                                           left_source='ocr_paragraph', right_source='ocr_same_row')},
                       limits=dict(max_proposals=2, max_text_bytes=512))
        context, schema, kind = provider.prepare(request, 16)
        self.assertEqual(context['occurrences'][0]['text_with_gap'], 'met [REDACTED] at the station')
        self.assertEqual(schema['properties']['proposals']['maxItems'], 2)
        self.assertNotIn('existing_texts', context)
        answer = {'proposals': [{'target': 't', 'text': ' Amber Vale '}, {'target': 't', 'text': 'Amber Vale'}]}
        result = provider.normalize(request, answer, 16, kind, 'local:test')
        self.assertEqual(result, {'schema_version': 1, 'proposals': [{'target': 't', 'text': 'Amber Vale'}]})
        for proposal in [{'target': 'unknown', 'text': 'Amber Vale'},
                         {'target': 't', 'text': 'Amber Vale', 'score': 1},
                         {'target': 't', 'text': '\n'}]:
            with self.assertRaises(ValueError):
                provider.normalize(request, {'proposals': [proposal]}, 16, kind, 'local:test')
        request['contexts']['t'] = dict(left='anti', right='ism', left_source='supplied', right_source='supplied')
        context, _, _ = provider.prepare(request, 8)
        self.assertEqual(context['occurrences'][0]['text_with_gap'], 'anti[REDACTED]ism')

    def test_structured_request_provenance_and_failures(self):
        args = argparse.Namespace(count=4, context_size=8192, tokens=1024,
            endpoint='http://localhost:11434', timeout=30, model='llama3.2:3b',
            temperature=0.7, seed=42, threads=8, trace=None)
        response = {'response': '{"texts":["mail"]}', 'done': True, 'done_reason': 'stop'}
        tags = {'models': [{'name': args.model, 'digest': 'abc'}]}
        with patch.object(provider.LocalOllama, 'call', side_effect=[tags, response]) as call:
            result = provider.run(self.request, args)
            self.assertIn('@abc', result['candidates'][0]['source_ids'][0])
            body = call.call_args_list[1].args[1]
            self.assertFalse(body['stream'])
            self.assertEqual(body['format']['required'], ['texts'])
        with patch.object(provider.LocalOllama, 'call', side_effect=[tags, dict(response, done_reason='length')]):
            with self.assertRaisesRegex(ValueError, 'Incomplete'):
                provider.run(self.request, args)
        remote = copy.deepcopy(tags)
        remote['models'][0]['remote_host'] = 'https://ollama.com'
        with patch.object(provider.LocalOllama, 'call', return_value=remote) as call:
            with self.assertRaisesRegex(ValueError, 'local weights'):
                provider.run(self.request, args)
            self.assertEqual(call.call_count, 1)

    def test_context_budget_and_mixed_surface_rejected(self):
        mixed = copy.deepcopy(self.request)
        mixed['targets'].append({'surface_kind': 'surname'})
        with self.assertRaises(ValueError):
            provider.prepare(mixed, 4)
        args = argparse.Namespace(count=4, context_size=8192, tokens=1024)
        huge = copy.deepcopy(self.request)
        huge['targets'][0]['left_context'] = 'x'*8192
        with self.assertRaisesRegex(ValueError, 'context budget'):
            provider.run(huge, args)

    def test_model_metadata_controls_instruction_roles_and_override(self):
        args = argparse.Namespace(count=4, context_size=8192, tokens=1024,
            endpoint='http://localhost:11434', timeout=30, model='renamed:local',
            temperature=0, seed=42, threads=8, trace=None, system_placement='auto')
        response = {'response': '{"texts":["mail"]}', 'done': True, 'done_reason': 'stop'}
        tags = {'models': [{'name': args.model, 'digest': 'abc', 'details': {'family': 'gemma3'}}]}
        with patch.object(provider.LocalOllama, 'call', side_effect=[tags, response]) as call:
            provider.run(self.request, args)
            body = call.call_args_list[1].args[1]
            self.assertNotIn('system', body)
            self.assertTrue(body['prompt'].startswith(provider.SYSTEM + '\n'))
            self.assertIn('arrived by [REDACTED] before', body['prompt'])
        args.system_placement = 'separate'
        with patch.object(provider.LocalOllama, 'call', side_effect=[tags, response]) as call:
            provider.run(self.request, args)
            body = call.call_args_list[1].args[1]
            self.assertEqual(body['system'], provider.SYSTEM)
            self.assertFalse(body['prompt'].startswith(provider.SYSTEM))
        args.system_placement = 'auto'
        tags['models'][0]['details']['family'] = 'llama'
        with patch.object(provider.LocalOllama, 'call', side_effect=[tags, response]) as call:
            provider.run(self.request, args)
            self.assertEqual(call.call_args_list[1].args[1]['system'], provider.SYSTEM)

    def test_plain_gap_joint_context_and_calibrated_width_ranges(self):
        request = dict(schema_version=1, task='propose_context_candidates', surface_kind='canonical',
            contexts={'b': dict(left='sent ', right=' today'), 'a': dict(left='read ', right=' yesterday')},
            limits=dict(max_proposals=8, max_text_bytes=512),
            width_guidance={'a': dict(observed_span_pixels=[80, 90], references=[
                dict(replacement='word', predicted_span_pixels=[30.123, 32.456], sigma_pixels=[1, 2])])})
        context, schema, kind = provider.prepare(request, 16, plain_gap=True)
        self.assertEqual(context['passages'], ['read [REDACTED] yesterday', 'sent [REDACTED] today'])
        self.assertEqual(schema['properties']['texts']['maxItems'], 8)
        self.assertNotIn('occurrences', context)
        self.assertEqual(context['approximate_widths_by_passage'][0]['references'][0]['predicted_span_pixels'], [30.1, 32.5])
        self.assertIsNone(context['approximate_widths_by_passage'][1])
        result = provider.normalize(request, {'texts': ['a letter', 'sent a report today']}, 8, kind, 'x', True)
        self.assertEqual(result['proposals'], [{'target': 'a', 'text': 'a letter'}, {'target': 'a', 'text': 'a report'}])
        self.assertEqual(result['context_targets_used'], ['a', 'b'])
        for interval in [[90, 80], [True, 3], [1, float('nan')], [1]]:
            request['width_guidance']['a']['observed_span_pixels'] = interval
            with self.assertRaises(ValueError):
                provider.prepare(request, 8, plain_gap=True)


if __name__ == '__main__':
    unittest.main()
