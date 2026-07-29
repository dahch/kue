import { t, useLanguage, type Language } from "./i18n";
import { Icon } from "./Icon";

interface HeaderProps {
  onLanguageChange?: (lang: Language) => void;
  onOpenSettings?: () => void;
}

function Logo() {
  return (
    <div className="flex items-center gap-3">
      <div className="relative">
        <img
          src="/kue-icon.svg"
          alt=""
          className="h-9 w-9 rounded-xl shadow-card ring-1 ring-white/10"
        />
        <span
          aria-hidden="true"
          className="absolute -right-0.5 -top-0.5 h-2 w-2 rounded-full bg-volt-400 ring-2 ring-ink-950"
        />
      </div>
      <div className="leading-tight">
        <h1 className="text-lg font-bold tracking-tight text-white">{t("appTitle")}</h1>
        <p className="font-mono text-[10px] uppercase tracking-[0.22em] text-zinc-500">
          {t("tagline")}
        </p>
      </div>
    </div>
  );
}

export default function Header({ onLanguageChange, onOpenSettings }: HeaderProps) {
  const language = useLanguage();

  return (
    <header className="sticky top-0 z-40 flex w-full items-center justify-between border-b border-white/5 bg-ink-950/75 px-6 py-3.5 backdrop-blur-xl">
      <Logo />
      <div className="flex items-center gap-2.5">
        <button
          type="button"
          aria-label={t("settings")}
          onClick={onOpenSettings}
          className="flex h-8 w-8 items-center justify-center rounded-lg text-zinc-500 transition-colors hover:bg-white/5 hover:text-white"
        >
          <Icon name="sliders" className="h-4 w-4" />
        </button>
        <Icon name="globe" className="h-4 w-4 text-zinc-500" />
        <div
          role="group"
          aria-label={t("selectLanguage")}
          className="flex rounded-lg border border-white/10 bg-ink-800 p-0.5"
        >
          {(["es", "en"] as const).map((lang) => (
            <button
              key={lang}
              type="button"
              aria-pressed={language === lang}
              onClick={() => onLanguageChange?.(lang)}
              className={`rounded-md px-2.5 py-1 font-mono text-[11px] font-medium uppercase tracking-wider transition-colors ${
                language === lang
                  ? "bg-volt-400 text-ink-950"
                  : "text-zinc-400 hover:text-zinc-200"
              }`}
            >
              {lang}
            </button>
          ))}
        </div>
      </div>
    </header>
  );
}
