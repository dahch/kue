import { useCallback, useEffect, useId, useRef, useState } from "react";
import { Icon } from "./Icon";

/* ---------- Spinner ---------- */

export function Spinner({ className = "h-4 w-4" }: { className?: string }) {
  return (
    <span
      role="status"
      aria-hidden="true"
      className={`inline-block animate-spin rounded-full border-2 border-zinc-500 border-t-volt-400 ${className}`}
    />
  );
}

/* ---------- Section label ---------- */

export function SectionLabel({ children }: { children: React.ReactNode }) {
  return (
    <h2 className="flex items-center gap-2 font-mono text-[11px] font-medium uppercase tracking-[0.18em] text-zinc-400">
      <span aria-hidden="true" className="h-1.5 w-1.5 rounded-[2px] bg-volt-400" />
      {children}
    </h2>
  );
}

/* ---------- Equalizer (live indicator) ---------- */

export function Equalizer({ className = "h-3.5 w-4", barClass = "bg-volt-400" }: { className?: string; barClass?: string }) {
  return (
    <span aria-hidden="true" className={`inline-flex items-end gap-[2.5px] ${className}`}>
      {[0, 1, 2, 3].map((i) => (
        <span
          key={i}
          className={`w-[2.5px] origin-bottom rounded-full animate-eq-bar ${barClass}`}
          style={{ height: "100%", animationDelay: `${i * 0.15}s`, animationDuration: `${0.9 + i * 0.12}s` }}
        />
      ))}
    </span>
  );
}

/* ---------- StyledSelect (accessible listbox) ---------- */

interface Option {
  value: string;
  label: string;
}

export function StyledSelect({
  value,
  onChange,
  options,
  className,
  ariaLabel,
}: {
  value: string;
  onChange: (value: string) => void;
  options: Option[];
  className?: string;
  ariaLabel?: string;
}) {
  const [open, setOpen] = useState(false);
  const activeIndexRef = useRef(0);
  const ref = useRef<HTMLDivElement>(null);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const optionRefs = useRef<(HTMLButtonElement | null)[]>([]);
  const listboxId = useId();
  const selected = options.find((o) => o.value === value);

  useEffect(() => {
    const handler = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, []);

  const openList = useCallback(() => {
    const idx = Math.max(0, options.findIndex((o) => o.value === value));
    activeIndexRef.current = idx;
    setOpen(true);
    requestAnimationFrame(() => optionRefs.current[idx]?.focus());
  }, [options, value]);

  const focusOption = useCallback((idx: number) => {
    activeIndexRef.current = idx;
    optionRefs.current[idx]?.focus();
  }, []);

  const selectAt = useCallback(
    (idx: number) => {
      const opt = options[idx];
      if (!opt) return;
      onChange(opt.value);
      setOpen(false);
      buttonRef.current?.focus();
    },
    [options, onChange],
  );

  const onButtonKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      if (!open) openList();
    }
  };

  const onListKeyDown = (e: React.KeyboardEvent) => {
    switch (e.key) {
      case "ArrowDown":
        e.preventDefault();
        focusOption(Math.min(options.length - 1, activeIndexRef.current + 1));
        break;
      case "ArrowUp":
        e.preventDefault();
        focusOption(Math.max(0, activeIndexRef.current - 1));
        break;
      case "Home":
        e.preventDefault();
        focusOption(0);
        break;
      case "End":
        e.preventDefault();
        focusOption(options.length - 1);
        break;
      case "Escape":
        e.preventDefault();
        setOpen(false);
        buttonRef.current?.focus();
        break;
      case "Tab":
        setOpen(false);
        break;
    }
  };

  return (
    <div ref={ref} className={`relative ${className ?? ""}`}>
      <button
        ref={buttonRef}
        type="button"
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-label={ariaLabel}
        onClick={() => (open ? setOpen(false) : openList())}
        onKeyDown={onButtonKeyDown}
        className="flex w-full items-center justify-between gap-2 rounded-lg border border-white/10 bg-ink-750 px-3 py-2 text-sm text-zinc-100 transition-colors hover:border-white/20"
      >
        <span className="truncate">{selected?.label ?? value}</span>
        <Icon
          name="chevron-down"
          className={`h-3.5 w-3.5 text-zinc-500 transition-transform duration-200 ${open ? "rotate-180" : ""}`}
        />
      </button>
      {open && (
        <div
          role="listbox"
          id={listboxId}
          aria-label={ariaLabel}
          onKeyDown={onListKeyDown}
          className="absolute z-[100] mt-1.5 max-h-60 w-full animate-scale-in overflow-y-auto rounded-lg border border-white/10 bg-ink-800 py-1 shadow-pop"
        >
          {options.map((o, i) => (
            <button
              key={o.value}
              ref={(el) => { optionRefs.current[i] = el; }}
              type="button"
              role="option"
              aria-selected={o.value === value}
              className={`flex w-full items-center justify-between px-3 py-2 text-left text-sm transition-colors ${
                o.value === value
                  ? "bg-volt-400/10 text-volt-400"
                  : "text-zinc-300 hover:bg-white/5"
              }`}
              onClick={() => selectAt(i)}
              onMouseEnter={() => { activeIndexRef.current = i; }}
            >
              {o.label}
              {o.value === value && <Icon name="check" className="h-3.5 w-3.5" />}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
