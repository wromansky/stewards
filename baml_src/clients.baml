// Placeholder client: `--direct` replaces it with a ClientRegistry built from ~/.stewards/config.toml,
// and the refresh test only renders requests from it.
client<llm> Default {
  provider openai
  options {
    model "gpt-4o-mini"
  }
}
