# FisoGen reference

[Back to the README](../README.md)

## Batch processing

1. Switch to **Pro** and choose **Batch** on **Create / Edit**.
2. Configure a prompt, quality, dimensions, image count, and optional references. Click **Add to batch**. This only adds to a draft; it makes no API call and needs no API key. You can also paste several prompts separated by a line containing only `---` to add several with the current settings.
3. Change the prompt, settings, or references and click **Add to Batch draft** again. Each added request keeps its settings and price snapshot. Review the queue, inspect request details, remove entries, or export the draft JSONL. Drafts stay in memory until you close the app. Each batch uses one model and one endpoint.
4. Confirm and click **Submit Batch job** to send the entire listed draft as one job. The form is not automatically included; add it first. The app uploads JSONL with `purpose=batch` and creates a job with a `24h` completion window. The submitted draft is cleared after success; requests cannot be added to an already submitted Batch. Choosing several images in Standard mode does not enable discounted Batch pricing.
5. Open **Batch Jobs**, then choose **Refresh status & collect results**. Inspect images and costs in **Gallery & History**. Results match requests by `custom_id`, and repeated collections preserve a single history entry per result.

Jobs and their rate snapshots survive restarts in `batches/`. Successful results from expired or cancelled jobs are imported too. Missing usage stays unknown. If submission times out, check the [Batch dashboard](https://platform.openai.com/batches); the app can attach the recovered Batch ID after verifying its input file. It never automatically resubmits an uncertain job. Output/error files expire after 30 days by default, so collect results promptly.

## Prices and cost details

The **Prices & cost planner** tab includes a searchable model/processing price table, links to official image pricing and the size/quality calculator, a Standard-versus-Batch workload comparison, per-request/per-image costs, and budget capacity. Planner token counts are user assumptions; they are not measured usage or guaranteed quotes.

Published snapshot verified **November 18, 2026**, for all four supported GPT Image 2.6/2.5 Sunburst/Flare models, in **USD per 1 million tokens**:

| Component | Standard | Batch |
| --- | ---: | ---: |
| Text input | $5 | $2.50 |
| Cached text input | $1.25 | $0.625 |
| Image input | $8 | $4 |
| Cached image input | $2 | $1 |
| Image output | $30 | $15 |

Sources: [official image prices](https://developers.openai.com/api/docs/pricing#image-generation), [cost calculator](https://developers.openai.com/api/docs/guides/image-generation#calculating-costs), [Batch guide](https://developers.openai.com/api/docs/guides/batch), and [caching eligibility](https://developers.openai.com/api/docs/guides/image-generation#cached-input-pricing).

**Refresh official prices** reads the official Markdown pricing tables without sending an API key. Refresh is session-local and only replaces the catalog when all expected rates parse successfully; otherwise existing rates remain. **Use custom rates** provides separate Standard/Batch overrides. Each request saves its mode, rates, source, verification timestamp, custom adjustment, raw usage, and calculated components. Changing the sidebar never reprices historical records.

- Costs use **actual API usage** when available: `tokens × rate / 1,000,000`. Output tokens already cover all images in the request and are never multiplied by `n` again.
- Text input, cached text, image input, cached image input, output, and custom adjustment are shown separately with token quantities, rates, and formulas. Average image cost divides by returned images, not requested images.
- Missing components remain unavailable. Known partial components are displayed, while incomplete requests are excluded from complete-spend totals and averages.
- GPT Image 2.5 cached input discounts apply only through the Responses API, not this app's direct Images API. For 2.6, reported cache modality details can be priced separately; absent cache data uses uncached rates, and unsplit reported cached usage makes the input estimate incomplete.
- Quality, size, references, and complexity can change consumption; equal token prices do not guarantee equal per-image cost.
- History supports model, processing, and quality filters, itemized request inspection, filtered CSV/JSONL exports, and individual complete-details JSON downloads. Old history remains readable with its original recorded costs.

## Limitations and local data

- Generation uses `/v1/images/generations`; references use `/v1/images/edits` with JSON image references. Batch uses the same endpoints through `/v1/batches`. Support and availability can depend on the account, model, and endpoint; API errors are displayed.
- `complexity` is sent only for 2.6 models. Transparent output requires PNG or WebP. At most 16 reference images are supported; Batch input is limited to 200 MB and 50,000 requests.
- The app uses the global OpenAI endpoint. Optional custom uplift is an accounting/planning adjustment, not a routing or billing setting. Invoices, regional uplifts, taxes, credits, negotiated rates, and undisclosed cache hits are not automatically detected. The [OpenAI usage dashboard](https://platform.openai.com/usage) remains authoritative.
- Prompts and reference filenames are stored locally. Reference images are included in requests sent to OpenAI.
- The app never writes your API key to history or job files. Billable actions require the confirmation checkbox and submit button. Refreshing prices and planning costs do not generate images.
- No API key or credit is included.
