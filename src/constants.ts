export interface ProviderOption {
  value: string;
  label: string;
}

export const PROVIDERS: ProviderOption[] = [
  { value: "openai", label: "OpenAI" },
  { value: "anthropic", label: "Anthropic" },
  { value: "gemini", label: "Gemini" },
  { value: "openrouter", label: "OpenRouter" },
  { value: "deepseek", label: "DeepSeek" },
  { value: "ollama", label: "Ollama (local)" },
];
