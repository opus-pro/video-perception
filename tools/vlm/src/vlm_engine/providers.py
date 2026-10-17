"""VLM provider backends managed by the user."""

from __future__ import annotations

import base64
import json
import os
import urllib.request
from typing import Protocol


class ModelBackend(Protocol):
    """Transport boundary implemented by local and remote model serving."""

    name: str

    def query(self, image_path: str, system_prompt: str, user_prompt: str, model: str) -> dict:
        """Run one structured vision inference request."""


def get_provider(backend: str | None = None) -> ModelBackend:
    """Return a provider based on VIDEO_PERCEPTION_VLM_BACKEND env var or explicit backend."""
    backend = backend or os.environ.get("VIDEO_PERCEPTION_VLM_BACKEND", "ollama")
    if backend == "gemini":
        return GeminiProvider()
    elif backend == "ollama":
        return OllamaProvider()
    config_json = os.environ.get("VIDEO_PERCEPTION_PROVIDER_CONFIG")
    if config_json:
        config = json.loads(config_json)
        if config.get("id") != backend:
            raise RuntimeError(
                "VIDEO_PERCEPTION_PROVIDER_CONFIG does not match selected backend"
            )
        runtime = config.get("runtime", {})
        runtime_type = runtime.get("type")
        if runtime_type == "openai-compatible":
            return OpenAICompatibleProvider(runtime)
        if runtime_type == "ollama-compatible":
            return OllamaProvider(_resolve_base_url(runtime, "http://localhost:11434"))
    raise ValueError(f"Unknown model backend: {backend}")


class OllamaProvider:
    """Local Ollama model-serving adapter."""

    name = "ollama"

    def __init__(self, base_url: str | None = None):
        self.base_url = base_url

    def query(self, image_path: str, system_prompt: str, user_prompt: str, model: str) -> dict:
        url = self.base_url or os.environ.get("OLLAMA_URL", "http://localhost:11434")
        with open(image_path, "rb") as f:
            img_b64 = base64.b64encode(f.read()).decode()

        messages = []
        if system_prompt:
            messages.append({"role": "system", "content": system_prompt})
        messages.append({"role": "user", "content": user_prompt, "images": [img_b64]})

        payload = json.dumps({
            "model": model,
            "messages": messages,
            "stream": False,
            "format": "json",
            "options": {"temperature": 0.1},
        }).encode()

        req = urllib.request.Request(
            f"{url}/api/chat",
            data=payload,
            headers={"Content-Type": "application/json"},
        )
        resp = urllib.request.urlopen(req, timeout=120)
        data = json.loads(resp.read())
        return json.loads(data["message"]["content"])


class OpenAICompatibleProvider:
    """User-declared OpenAI-compatible endpoint.

    The manifest carries only environment-variable names, never credential
    values. This adapter is intentionally model-list agnostic so a new model
    does not require a Video Perception release.
    """

    name = "openai-compatible"

    def __init__(self, runtime: dict):
        self.runtime = runtime
        self.base_url = _resolve_base_url(runtime, "")
        if not self.base_url:
            raise RuntimeError("OpenAI-compatible provider has no base URL")

    def query(self, image_path: str, system_prompt: str, user_prompt: str, model: str) -> dict:
        with open(image_path, "rb") as f:
            img_b64 = base64.b64encode(f.read()).decode()

        text = f"{system_prompt}\n\n{user_prompt}" if system_prompt else user_prompt
        payload = json.dumps({
            "model": model,
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "text", "text": text},
                    {
                        "type": "image_url",
                        "image_url": {"url": f"data:image/png;base64,{img_b64}"},
                    },
                ],
            }],
            "temperature": 0.1,
            "response_format": {"type": "json_object"},
        }).encode()

        headers = {"Content-Type": "application/json"}
        credential_env = self.runtime.get("credential_env")
        if credential_env:
            credential = os.environ.get(credential_env)
            if not credential:
                raise RuntimeError(f"credential environment variable {credential_env} is not set")
            headers["Authorization"] = f"Bearer {credential}"

        request = urllib.request.Request(
            f"{self.base_url.rstrip('/')}/chat/completions",
            data=payload,
            headers=headers,
        )
        response = urllib.request.urlopen(request, timeout=120)
        data = json.loads(response.read())
        text = data["choices"][0]["message"]["content"].strip()
        if text.startswith("```"):
            text = text.strip("`")
            if text.startswith("json"):
                text = text[4:]
        return json.loads(text.strip())


def _resolve_base_url(runtime: dict, default: str) -> str:
    env_name = runtime.get("base_url_env")
    if env_name and os.environ.get(env_name):
        return os.environ[env_name].rstrip("/")
    return str(runtime.get("base_url") or default).rstrip("/")


class GeminiProvider:
    """Google Gemini via REST API (no SDK dependency)."""

    name = "gemini"

    def query(self, image_path: str, system_prompt: str, user_prompt: str, model: str) -> dict:
        api_key = os.environ.get("GEMINI_API_KEY")
        if not api_key:
            raise RuntimeError(
                "GEMINI_API_KEY not set for model_backend provider=gemini"
            )

        with open(image_path, "rb") as f:
            img_b64 = base64.b64encode(f.read()).decode()

        parts = []
        if system_prompt:
            parts.append({"text": system_prompt + "\n\n" + user_prompt})
        else:
            parts.append({"text": user_prompt})
        parts.append({
            "inline_data": {"mime_type": "image/png", "data": img_b64},
        })

        payload = json.dumps({
            "contents": [{"parts": parts}],
            "generationConfig": {
                "temperature": 0.1,
                "responseMimeType": "application/json",
            },
        }).encode()

        url = (
            f"https://generativelanguage.googleapis.com/v1beta/models/{model}"
            f":generateContent?key={api_key}"
        )
        req = urllib.request.Request(url, data=payload, headers={"Content-Type": "application/json"})
        resp = urllib.request.urlopen(req, timeout=120)
        data = json.loads(resp.read())

        text = data["candidates"][0]["content"]["parts"][0]["text"]
        return json.loads(text)
