# Contributing to Video Perception

Thanks for helping agents understand video through transparent, user-controlled tools.

## Before opening a change

- Search existing issues and keep one pull request focused on one concern.
- User and project Providers, Strategies, and Knowledge Packs do not require a
  core pull request. Publish them independently and validate them with
  `video-perception extension validate`.
- For a new standard capability or built-in implementation, open a design issue
  first and describe its input, output, locality, prerequisites, failure
  behavior, and streaming strategy.
- Do not add company credentials, private endpoints, consumer profiles, model weights, or copyrighted test media.

## Development setup

```bash
npm ci
cd cli && cargo test --all-targets
cd ../tools/vlm && uv run --isolated --no-project --with pytest --with . --python 3.11 python -m pytest tests -v
cd ../..
npm run schema:check
npm run schema:export
git diff --exit-code schema/json
```

FFmpeg, ffprobe, and ImageMagick are required for end-to-end CLI tests. Python helper tools target Python 3.11 or newer.

## Adding a capability

Update all affected layers:

1. capability metadata and dependencies in `cli/src/registry.rs`;
2. execution adapter and output provenance;
3. TypeScript contract and generated JSON schema;
4. catalog, planner, Observation, and failure-path tests;
5. streaming metadata, even when the adapter is not implemented;
6. public documentation and the Agent Skill.

Provider errors must never be reported as successful empty evidence. Discovery and setup must remain read-only and must not install software or read credentials.

## Authoring extensions

Read [docs/extensions.md](docs/extensions.md). A Provider contribution must
declare only the capability cells it implements, keep credential values out of
the manifest, and include safe fixtures for output validation. A Strategy must
collect evidence rather than perform a final business action. A Knowledge Pack
must not redistribute tutorials, datasets, model weights, or documentation
without an appropriate license.

## Pull requests

Use a clear title, explain the user-visible behavior, list verification commands, and call out privacy or compatibility changes. Add a decision record under `docs/decisions/` for durable architectural choices and a postmortem under `docs/postmortems/` for material incidents.

## Releasing

Push an annotated tag `vX.Y.Z` on `main`. The Release workflow builds the binaries, wheels and plugin bundle, uploads them as the `release-assets` artifact, and then creates the GitHub Release with `checksums.txt`. If that last step cannot run from the hosted runner, publish the same artifact from your machine:

```sh
scripts/publish-release.sh vX.Y.Z
```

The script verifies `checksums.txt` before it creates the release, or replaces the assets when the release already exists. The `publish-npm` and `update-homebrew-tap` jobs depend on the workflow's own release step and are skipped while `NPM_TOKEN` and `HOMEBREW_TAP_TOKEN` are unset.

By contributing, you agree that your contribution is licensed under the MIT License.
