# Deployment

Want a general hosting recommendation? [Just fucking use Cloudflare](https://justfuckingusecloudflare.com/). ReGen produces files. You do not need to invent a platform to serve them.

## Build and upload

1. Set `site.base_url` to your site's public URL, including any case-sensitive prefix: `https://example.com/` or `https://example.com/project/`.
2. Build with your [ReGen installation](guide.md#installation):

   ```sh
   regen build --site .
   ```

3. Upload **only `dist/`**, not the source, templates or build files. Save a copy of the previous site if you may need to roll back.

`base_url` changes browser URLs, not disk paths: `/project/about/` still comes from `dist/about/index.html`. Configure your host to [serve the files at that prefix](architecture.md#hosting-paths-are-not-disk-paths). TOML does not expand environment variables; if CI supplies the URL, write it into `regen.toml` before building.

## Cloudflare Workers with Static Assets

Use an **assets-only Worker** to serve `dist/` without an edge script. This example serves the site at the domain root.

Install Node.js **22+** and Wrangler **4.129.0** in your deployment workspace:

```sh
npm install --save-dev --save-exact wrangler@4.129.0
```

Keep `package.json` and `package-lock.json`; use `npm ci` to install the same versions in CI. `npx --no-install` uses the installed Wrangler rather than downloading another version. ReGen itself does not need these tools.

Put `wrangler.jsonc` beside `regen.toml`, choosing an unused Worker name you control:

```json
{
  "name": "regen-static-site",
  "compatibility_date": "2026-09-03",
  "workers_dev": true,
  "assets": {
    "directory": "./dist",
    "html_handling": "force-trailing-slash",
    "not_found_handling": "none"
  }
}
```

`/about` redirects to `/about/`, serving `dist/about/index.html`. Unknown paths return **404**. Do not enable an SPA fallback for a multi-page site.

Build for `http://127.0.0.1:8787/` when you want local canonical URLs, then preview:

```sh
regen build --site .
npx --no-install wrangler dev --local --ip 127.0.0.1 --port 8787
```

Stop the preview. Set `site.base_url` to your public `workers.dev` or custom-domain URL, then rebuild and check the upload without sending it:

```sh
regen build --site .
npx --no-install wrangler deploy --dry-run --outdir .wrangler/dry-run
```

When ready, sign in to Cloudflare, check the account and Worker name, and run `npx --no-install wrangler deploy`. This replaces the Worker's deployed site. The dry run above checks the local upload; DNS, HTTPS and routing still need checking on the live site.

Use different Worker names for preview and production so a preview cannot overwrite the live site. A secret URL and `noindex` do not restrict access.

For other hosting options, see Cloudflare's documentation:

- [Static Assets configuration](https://developers.cloudflare.com/workers/static-assets/configuration/), [Custom Domains](https://developers.cloudflare.com/workers/configuration/routing/custom-domains/), and [Routes](https://developers.cloudflare.com/workers/configuration/routing/routes/). Matching a route prefix does **not** strip it; use an explicit [subdirectory mount/rewrite](https://developers.cloudflare.com/workers/static-assets/routing/advanced/serving-a-subdirectory/).
- [Headers and caching](https://developers.cloudflare.com/workers/static-assets/headers/): let browsers recheck HTML; use immutable caching only for content-hashed assets. `_headers` applies to static asset responses, not responses from custom Worker code.
- [CI authentication](https://developers.cloudflare.com/workers/ci-cd/external-cicd/github-actions/) and [rollbacks](https://developers.cloudflare.com/workers/versions-and-deployments/rollbacks/). Rolling back a Worker version does not undo DNS or other account settings.

## GitHub Pages

In your site's repository, select **Settings → Pages → Source → GitHub Actions**. Follow GitHub's [custom workflow guide](https://docs.github.com/en/pages/getting-started-with-github-pages/using-custom-workflows-with-github-pages) to build with ReGen, upload `dist/`, and deploy it with the Pages actions.

Set `site.base_url` to the Pages URL. For a project site such as `https://example.github.io/my-site/`, include `/my-site/` in the URL but upload the contents of `dist/` directly — do not create another `my-site/` directory inside it.

Configure custom-domain DNS/HTTPS through [GitHub's hosting documentation](https://docs.github.com/en/pages/configuring-a-custom-domain-for-your-github-pages-site). Pages does not interpret Cloudflare `_headers` or `_redirects`.

## Before sharing the link

- Inspect `dist/` for private files and media metadata. ReGen copies `public/` files unchanged and does not remove personal information from images.
- Keep credentials and local deployment files such as `.dev.vars` out of your source and output. Store CI tokens in the hosting platform's secret settings, with access limited to the site being deployed.
- Open the deployed site and check nested pages, language links, canonical URLs, trailing-slash redirects, 404s, fonts and other assets.
- Keep the previous deployment available until the new one works.
