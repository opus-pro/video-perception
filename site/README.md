# Landing page

A single static page: `index.html` plus the generated `results.js`. It needs no
build step and loads only Google Fonts from outside the repository.

Every number on the page comes from the evaluation in `evals/perception`.
After a new evaluation run, regenerate the data:

```sh
python3 evals/perception/grade.py evals/perception/results/raw/*.jsonl \
  --out evals/perception/results/summary.md --graded evals/perception/results/graded.jsonl
python3 site/build_results.py
```

`story.json` picks the two runs shown side by side and describes their steps in
plain words. Times and outcomes are read from the graded runs, not typed in.

Preview locally:

```sh
python3 -m http.server 8765 --directory site
```

The page is static, so any static host can serve the `site/` directory as is.
