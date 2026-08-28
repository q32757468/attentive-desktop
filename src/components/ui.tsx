import type {
  ButtonHTMLAttributes,
  HTMLAttributes,
  InputHTMLAttributes,
  ReactNode,
  TextareaHTMLAttributes,
} from "react";

const CONTROL_BASE_CLASS =
  "w-full min-w-0 rounded-lg border border-slate-200 bg-white text-sm text-slate-800 shadow-sm transition placeholder:text-slate-400 focus:border-blue-500 focus:outline-none focus:ring-4 focus:ring-blue-100 disabled:cursor-not-allowed disabled:bg-slate-50";

const BUTTON_BASE_CLASS =
  "inline-flex h-9 items-center justify-center gap-2 rounded-lg px-3 text-sm font-medium transition disabled:cursor-not-allowed disabled:opacity-60";

type CardProps = HTMLAttributes<HTMLElement>;

export function Card({ className = "", children, ...props }: CardProps): ReactNode {
  return (
    <section
      {...props}
      className={`rounded-[11px] border border-slate-200 bg-white shadow-[0_1px_4px_rgba(28,45,75,0.04)] ${className}`}
    >
      {children}
    </section>
  );
}

type SectionHeaderProps = {
  id: string;
  icon: ReactNode;
  title: string;
  className?: string;
};

export function SectionHeader({ id, icon, title, className = "" }: SectionHeaderProps): ReactNode {
  return (
    <div className={`flex items-center gap-3 ${className}`}>
      {icon}
      <h2 id={id} className="text-[20px] font-semibold tracking-[-0.02em] text-slate-900">{title}</h2>
    </div>
  );
}

type FormFieldProps = {
  label: string;
  children: ReactNode;
  className: string;
  align?: "center" | "start";
  labelClassName?: string;
};

export function FormField({
  label,
  children,
  className,
  align = "center",
  labelClassName = "",
}: FormFieldProps): ReactNode {
  const alignmentClass = align === "start" ? "items-start" : "items-center";

  return (
    <label className={`grid ${alignmentClass} gap-3 ${className}`}>
      <span className={`text-sm text-slate-700 ${labelClassName}`}>{label}</span>
      {children}
    </label>
  );
}

type TextInputProps = InputHTMLAttributes<HTMLInputElement>;

export function TextInput({ className = "", ...props }: TextInputProps): ReactNode {
  return <input {...props} className={`${CONTROL_BASE_CLASS} h-10 px-3 ${className}`} />;
}

type TextAreaProps = TextareaHTMLAttributes<HTMLTextAreaElement>;

export function TextArea({ className = "", ...props }: TextAreaProps): ReactNode {
  return (
    <textarea
      {...props}
      className={`${CONTROL_BASE_CLASS} min-h-[132px] resize-y px-3 py-2 leading-6 ${className}`}
    />
  );
}

type ButtonProps = ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: "primary" | "secondary";
};

export function Button({ variant = "primary", className = "", ...props }: ButtonProps): ReactNode {
  const variantClass = variant === "secondary"
    ? "border border-slate-200 bg-white text-slate-700 hover:border-slate-300 hover:bg-slate-50"
    : "bg-[#1769e8] text-white shadow-[0_4px_10px_rgba(23,105,232,0.2)] hover:bg-[#0f5fd8]";

  return <button {...props} className={`${BUTTON_BASE_CLASS} ${variantClass} ${className}`} />;
}
