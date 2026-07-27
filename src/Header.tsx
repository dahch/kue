import { t, useLanguage, type Language } from "./i18n";

interface HeaderProps {
  onLanguageChange?: (lang: Language) => void;
}

function Logo() {
  return (
    <div className="flex items-center gap-3">
      <img
        src="/kue-icon.svg"
        alt="Kue"
        className="h-10 w-10 rounded-xl shadow-lg"
      />
      <div>
        <h1 className="text-xl font-bold tracking-tight text-white">{t("appTitle")}</h1>
        <p className="text-xs text-zinc-500">{t("tagline")}</p>
      </div>
    </div>
  );
}

export default function Header({ onLanguageChange }: HeaderProps) {
  const language = useLanguage();

  return (
    <header className="flex w-full items-center justify-between border-b border-zinc-800 bg-zinc-950/80 px-6 py-4 backdrop-blur">
      <Logo />
      <div className="flex items-center gap-3">
        <label className="text-xs text-zinc-500">{t("language")}</label>
        <select
          value={language}
          onChange={(e) => {
            onLanguageChange?.(e.target.value as Language);
          }}
          className="rounded-lg border border-zinc-700 bg-zinc-900 px-2 py-1 text-sm text-white outline-none focus:border-blue-500"
        >
          <option value="es">Español</option>
          <option value="en">English</option>
        </select>
      </div>
    </header>
  );
}
