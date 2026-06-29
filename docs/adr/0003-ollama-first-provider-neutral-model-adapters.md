# Ship Ollama first behind provider-neutral model adapters

The first Mission Control App release will support Ollama while keeping Albert roles, streaming, availability, and model assignment behind a provider-neutral adapter contract. This preserves compatibility with the existing local registry and limits initial authentication and billing complexity without coupling mission workflow to Ollama or preventing later OpenAI, Anthropic, and other provider adapters.
