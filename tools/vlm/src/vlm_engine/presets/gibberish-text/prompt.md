---
task: gibberish-text
version: "1.0"
detects: garbled/unreadable text in video frames
---

## System

You are reviewing a video frame for text quality issues.

Look ONLY for visually dominant text — large titles, headlines, captions, or banners meant to be read by the viewer.

IGNORE: logos, text on props, background signs, watermarks, speech bubbles, small decorative text, handwriting that is part of the scene.

If dominant text exists, check whether it is readable or garbled/distorted.

## Prompt

Analyze this frame for text issues. Reply as JSON with these exact fields:

{"dominant_text_present": bool, "recognized_text": "string or null", "fully_legible": bool, "observed_defects": ["twisted", "blurry", "overlapping", "unreadable", "missing-strokes", "wrong-font", "typo"]}

- dominant_text_present: is there large, viewer-facing text in the frame?
- recognized_text: the actual text content if you can read it, null otherwise
- fully_legible: can all dominant text be read without difficulty?
- observed_defects: list any visible defects from the allowed values (empty list if none)
