from vlm_engine.providers import get_provider
import json
import pytest


def test_registered_model_backends_expose_stable_names():
    assert get_provider("ollama").name == "ollama"
    assert get_provider("gemini").name == "gemini"


def test_default_backend_is_local(monkeypatch):
    monkeypatch.delenv("VIDEO_PERCEPTION_VLM_BACKEND", raising=False)
    assert get_provider().name == "ollama"


def test_unknown_model_backend_is_rejected():
    with pytest.raises(ValueError, match="Unknown model backend"):
        get_provider("consumer-owned-profile")


def test_user_openai_compatible_provider_does_not_require_a_central_model_list(monkeypatch):
    monkeypatch.setenv(
        "VIDEO_PERCEPTION_PROVIDER_CONFIG",
        json.dumps({
            "id": "user.my-vision",
            "runtime": {
                "type": "openai-compatible",
                "base_url": "https://example.com/v1",
                "credential_env": "MY_VISION_KEY",
            },
        }),
    )
    provider = get_provider("user.my-vision")
    assert provider.name == "openai-compatible"
    assert provider.base_url == "https://example.com/v1"
