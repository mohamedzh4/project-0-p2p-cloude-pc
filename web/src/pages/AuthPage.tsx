import { AuthForm } from "../components/AuthForm";
import { Brand } from "../components/Brand";
import type { AuthMode } from "../types";

interface AuthPageProps {
  mode: AuthMode;
  busy: boolean;
  email: string;
  password: string;
  error: string | null;
  reason: string | null;
  onMode: (mode: AuthMode) => void;
  onEmail: (value: string) => void;
  onPassword: (value: string) => void;
  onSubmit: () => void;
}

export function AuthPage(props: AuthPageProps) {
  return (
    <div className="auth">
      <aside className="auth-art">
        <img src="/img/hero-create.webp" alt="" width={1024} height={572} />
        <div className="auth-art-copy">
          <p className="auth-quote">A render farm, a game rig, a quiet place to code. Borrowed for the evening.</p>
        </div>
      </aside>
      <main className="auth-main">
        <div className="auth-top">
          <Brand />
        </div>
        <AuthForm {...props} />
        <p className="auth-foot">Your sign-in lasts until you close this tab.</p>
      </main>
    </div>
  );
}
