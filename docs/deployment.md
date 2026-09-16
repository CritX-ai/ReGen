# Deployment

Want a general hosting recommendation? [Just fucking use Cloudflare](https://justfuckingusecloudflare.com/). ReGen produces files; your host serves them.

## Build and upload

1. Set `site.base_url` to the public URL, including any case-sensitive project prefix. TOML does not expand environment variables.
2. [Install ReGen](releasing.md), then build from your site directory:

   ```sh
   regen build --site . --profile release
   ```

3. Upload **only `dist/`**, not source or build files. Keep the previous deployment for rollback.

A URL prefix does not add a disk directory: `/project/about/` still comes from `dist/about/index.html`. Your host must [serve the files at that prefix](architecture.md#hosting-paths-are-not-disk-paths). To check release settings without replacing `dist/`, use [review mode](reference.md#review-the-final-profile). Configured hooks still run.

<details class="inline-details" id="local-canonical-urls">
<summary>Local canonical URLs</summary>

Set `site.base_url` to your local server before building: `http://127.0.0.1:8000/` for the [Caddy preview](guide.md#preview-locally-with-caddy), or `http://127.0.0.1:8787/` for Wrangler. Match any hosting prefix to the server's mount. Restore the public URL and rebuild before uploading—`--review` changes the output directory, not canonical URLs.

</details>

## Cloudflare Workers with Static Assets

Use an **assets-only Worker**. Copy [wrangler.jsonc](../examples/deployment/wrangler.jsonc) and [cloudflare-workers.sh](../examples/deployment/cloudflare-workers.sh) beside `regen.toml`. Choose your Worker name and set `site.base_url` to its `workers.dev` or custom-domain URL. This example serves at the domain root.

Run commands as `sh cloudflare-workers.sh COMMAND`:

1. `setup` once; commit the generated `package.json` and lockfile.
2. `install` in CI (`npm ci`), then `build`.
3. `preview` locally or `dry-run` to prepare an upload without publishing.
4. `deploy` when you're ready to replace the live site.

The script pins [Wrangler 4](https://developers.cloudflare.com/workers/wrangler/) and needs [Node.js 22+](https://nodejs.org/) with npm.

In CI, store `CLOUDFLARE_ACCOUNT_ID` and a site-scoped `CLOUDFLARE_API_TOKEN` in secret settings; locally, use Wrangler login. Check the account and Worker name before deploying. Use different names for preview and production—a secret URL or `noindex` is not access control.

`force-trailing-slash` redirects `/about` to `/about/`; `not_found_handling: none` gives real **404s**, not an SPA fallback. A dry run prepares files, but cannot check DNS, HTTPS or live routing.

- [Static Assets configuration](https://developers.cloudflare.com/workers/static-assets/configuration/) · [Custom Domains](https://developers.cloudflare.com/workers/configuration/routing/custom-domains/) · [Routes](https://developers.cloudflare.com/workers/configuration/routing/routes/). Route prefixes are not stripped automatically: use an explicit [subdirectory mount/rewrite](https://developers.cloudflare.com/workers/static-assets/routing/advanced/serving-a-subdirectory/).
- [Headers and caching](https://developers.cloudflare.com/workers/static-assets/headers/): revalidate HTML; reserve immutable caching for hashed assets. `_headers` applies to static assets, not custom Worker responses.
- [CI authentication](https://developers.cloudflare.com/workers/ci-cd/external-cicd/github-actions/) · [Rollbacks](https://developers.cloudflare.com/workers/versions-and-deployments/rollbacks/); rollbacks do not undo account or DNS changes.

## GitHub Pages

Copy [the workflow](../examples/deployment/github-pages.yml) to `.github/workflows/pages.yml`, then choose **Settings → Pages → Source → GitHub Actions** in your repository. It pins Rust and ReGen, builds `dist/` and deploys only the default branch. Installation and publication need network access.

Set `site.base_url` to your Pages URL, including `/my-site/` for a project such as `https://example.github.io/my-site/`. See GitHub's [workflow](https://docs.github.com/en/pages/getting-started-with-github-pages/using-custom-workflows-with-github-pages) and [custom-domain guides](https://docs.github.com/en/pages/configuring-a-custom-domain-for-your-github-pages-site). Pages ignores Cloudflare's `_headers` and `_redirects`.

## GitLab Pages

Copy [the workflow](../examples/deployment/gitlab-pages.yml) to `.gitlab-ci.yml` beside `regen.toml`. It needs **GitLab 17.10+**, Pages enabled, and a networked Linux **amd64 or arm64** Docker/Kubernetes runner. The Rust image supplies the compiler and linker; shell executors ignore `image:` and need those tools installed. Only default-branch pipelines deploy.

Use the URL under **Deploy → Pages**, or your [custom domain](https://docs.gitlab.com/user/project/pages/custom_domains_ssl_tls_certification/), for `site.base_url`. Namespace URLs may include project and subgroup prefixes; [unique domains](https://docs.gitlab.com/user/project/pages/#unique-domains) serve at the root and are enabled by default. Rebuild after changing the URL; TOML does not expand `$CI_PAGES_URL`.

[`pages.publish: dist`](https://docs.gitlab.com/ci/yaml/#pagespublish) adds the output to job artifacts automatically on GitLab 17.10+. Keep `public/` as ReGen's **input** directory and `dist/` as output. Pages needs a nonempty `dist/index.html`. The [Pages URL reference](https://docs.gitlab.com/user/project/pages/getting_started_part_one/) covers other layouts.

