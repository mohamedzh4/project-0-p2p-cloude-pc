import { PixelIcon } from "../components/PixelIcon";

import { useEffect, useState, type ReactNode } from "react";

import type { HostKitOs } from "../api";
import { Brand } from "../components/Brand";
import { SegmentBar } from "../components/SegmentBar";
import { SkyCanvas } from "../components/SkyCanvas";
import { routeHref } from "../router";

interface LandingProps {
  signedIn: boolean;
  onDownload: (os: HostKitOs) => void;
}

const FAQ: { q: string; a: string }[] = [
  {
    q: "Do I need to install anything to use a computer?",
    a: "Not for Standard. It opens in a browser tab. Pro streams a whole graphics card, so you open it with Moonlight, a free streaming app for Windows, macOS, Linux, iOS and Android.",
  },
  {
    q: "Are my files kept after a session ends?",
    a: "Files you save in the Persistent folder are kept and come back in your next session. Anything saved elsewhere on the desktop is gone when the session ends.",
  },
  {
    q: "Can I keep using my PC while I lend it?",
    a: "Yes. You choose how many cores and how much memory to lend, and the rest stays yours. If you lend your graphics card for Pro, you can't use that card while someone is connected.",
  },
  {
    q: "What can host a session?",
    a: "A Windows 10 or 11 PC, or Linux (Ubuntu, Debian or Arch), with hardware virtualization turned on. Sessions need at least 2 cores and 4 GiB of memory to spare.",
  },
  {
    q: "What are credits?",
    a: "Credits count how much time you have used and lent. They are kept on the lab's own ledger. Nothing is charged to a card, and credits are not paid out.",
  },
];

export function Landing({ signedIn, onDownload }: LandingProps) {
  const [scrolled, setScrolled] = useState(false);
  useEffect(() => {
    const onScroll = () => setScrolled(window.scrollY > window.innerHeight * 0.85);
    onScroll();
    window.addEventListener("scroll", onScroll, { passive: true });
    return () => window.removeEventListener("scroll", onScroll);
  }, []);

  // Everything below the hero fades in as it scrolls into view.
  useEffect(() => {
    const targets = document.querySelectorAll<HTMLElement>(".landing [data-reveal]");
    if (window.matchMedia("(prefers-reduced-motion: reduce)").matches || !("IntersectionObserver" in window)) {
      targets.forEach((el) => el.setAttribute("data-reveal", "shown"));
      return;
    }
    const observer = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          if (entry.isIntersecting) {
            entry.target.setAttribute("data-reveal", "shown");
            observer.unobserve(entry.target);
          }
        }
      },
      { rootMargin: "0px 0px -12% 0px", threshold: 0.12 },
    );
    targets.forEach((el) => observer.observe(el));
    return () => observer.disconnect();
  }, []);

  return (
    <div className="landing">
      <header className="topbar" data-scrolled={scrolled ? "true" : "false"}>
        <Brand />
        <nav className="navbox" aria-label="Main">
          <span className="navbox-links">
            <ScrollLink to="use">Use a PC</ScrollLink>
            <ScrollLink to="lend">Lend yours</ScrollLink>
            <ScrollLink to="downloads">Downloads</ScrollLink>
            <ScrollLink to="faq">FAQ</ScrollLink>
          </span>
          {signedIn ? (
            <a className="btn btn-solid" href={routeHref("sessions")}>
              Dashboard
            </a>
          ) : (
            <>
              <a href={routeHref("login")}>Log in</a>
              <a className="btn btn-solid" href={routeHref("signup")}>
                Get started
              </a>
            </>
          )}
        </nav>
      </header>

      <section className="hero" aria-labelledby="hero-title">
        <SkyCanvas />
        <h1 id="hero-title" className="hero-title">
          Let your low‑spec PC access a high‑performance virtual desktop.
        </h1>
      </section>

      <section className="band intro" aria-label="What Blue Sky does">
        <div className="intro-copy" data-reveal>
          <h2>Somebody's idle PC, open in your browser.</h2>
          <p className="intro-lede">
            Blue Sky connects you to real computers that people aren't using right now. Write code on a
            desktop in a tab, or take a whole graphics card for games, 3D and training.
          </p>
          <div className="intro-actions">
            <a className="btn btn-rent btn-lg" href={routeHref(signedIn ? "sessions" : "signup")}>
              <PixelIcon name="monitor" size={18} />
              Start a session
            </a>
            <ScrollLink to="lend" className="btn btn-outline btn-lg">
              Lend my PC
            </ScrollLink>
          </div>
        </div>
        <figure className="intro-figure" data-reveal>
          <img
            src="/img/hero-play.webp"
            alt="A laptop showing a game, with its characters stepping out of the screen"
            width={1024}
            height={564}
            loading="lazy"
          />
        </figure>
      </section>

      <section className="band" id="use">
        <div data-reveal className="band-intro">
          <h2>Two kinds of computer.</h2>
          <p>
            Tell us what you want to do and we pick the tier for you. You never browse hosts: the
            first free computer that fits is yours.
          </p>
        </div>
        <div data-reveal className="tiers">
          <article className="tier" data-tier="standard">
            <div className="tier-icon">
              <PixelIcon name="window" size={18} />
            </div>
            <h3>Standard</h3>
            <p className="tier-lead">A Linux desktop in a browser tab.</p>
            <p>
              A slice of a host's processor and memory for code, documents and the web. Nothing to
              install, and it opens in a minute or two.
            </p>
            <ul className="ticks">
              <li>Opens in any modern browser</li>
              <li>At least 2 cores and 4 GiB of memory</li>
              <li>Persistent folder kept between sessions</li>
            </ul>
          </article>
          <article className="tier tier-pro" data-tier="pro">
            <img
              className="tier-art"
              src="/img/hero-create.webp"
              alt="A 3D scene of a steam train and a beetle bursting out of a laptop running Blender"
              width={1024}
              height={572}
              loading="lazy"
            />
            <div className="tier-body">
              <div className="tier-icon">
                <PixelIcon name="gamepad" size={18} />
              </div>
              <h3>Pro</h3>
              <p className="tier-lead">A whole graphics card, just for you.</p>
              <p>
                The host's GPU is handed to your session alone for games, Blender, video work and
                model training. Stream it at full frame rate with Moonlight.
              </p>
              <ul className="ticks">
                <li>Exclusive GPU, not shared with the host</li>
                <li>Low-latency streaming with Moonlight</li>
                <li>Matched only to approved Pro hosts</li>
              </ul>
            </div>
          </article>
        </div>
      </section>

      <section className="band" aria-labelledby="steps-title">
        <div data-reveal className="band-intro">
          <h2 id="steps-title">From a sentence to a desktop.</h2>
        </div>
        <ol data-reveal className="steps">
          <li>
            <h3>Say what you'll do</h3>
            <p>"Write some Python" or "play a game with a dedicated GPU". That's how we pick the tier.</p>
          </li>
          <li>
            <h3>We find a free computer</h3>
            <p>An approved host with room to spare starts your desktop. You aren't charged while it boots.</p>
          </li>
          <li>
            <h3>Open it and work</h3>
            <p>Standard opens in a tab, Pro in Moonlight. End the session when you're done.</p>
          </li>
        </ol>
      </section>

      <section className="band band-lend" id="lend">
        <div data-reveal className="lend-copy">
          <h2>Your PC works while you don't.</h2>
          <p>
            Install Host Agent, choose what you can spare, and your computer takes sessions while you
            keep using it. Every minute it hosts earns you credits.
          </p>
          <ul className="features">
            <li>
              <PixelIcon name="pause" size={18} />
              <div>
                <h3>Pause any time</h3>
                <p>One click and your PC stops taking new sessions.</p>
              </div>
            </li>
            <li>
              <PixelIcon name="shield" size={18} />
              <div>
                <h3>Sealed off from your files</h3>
                <p>Guests run in a separate virtual machine, never on your own desktop.</p>
              </div>
            </li>
            <li>
              <PixelIcon name="server" size={18} />
              <div>
                <h3>Approved before it goes live</h3>
                <p>An operator checks each new host before any session lands on it.</p>
              </div>
            </li>
          </ul>
        </div>
        <div data-reveal className="agent-mock" aria-label="Host Agent sharing settings, as an example">
          <div className="agent-mock-bar">
            <span>Host Agent</span>
            <span className="agent-mock-state">Sharing</span>
          </div>
          <SegmentBar icon={<PixelIcon name="cpu" size={18} />} label="CPU cores" readout="6 of 16" filled={4}
            preview={(n) => `${Math.round((n / 10) * 16)} of 16`}
          />
          <SegmentBar icon={<PixelIcon name="memory" size={18} />} label="Memory" readout="12 of 32 GiB" filled={4}
            preview={(n) => `${Math.round((n / 10) * 32)} of 32 GiB`}
          />
          <SegmentBar icon={<PixelIcon name="gamepad" size={18} />} label="Graphics card" readout="Not lent" filled={0}
            preview={(n) => (n === 0 ? "Not lent" : "Lent for Pro")}
          />
          <p className="agent-mock-note">Earned this week: 148 credits</p>
        </div>
      </section>

      <section className="band" id="downloads">
        <div data-reveal className="band-intro">
          <h2>Downloads.</h2>
          <p>
            The Host Agent setup is made for your account, so we ask you to sign in first. The
            download starts right after.
          </p>
        </div>
        <div data-reveal className="downloads">
          <article className="download" data-tone="share">
            <PixelIcon name="windows" size={27} className="download-icon" />
            <h3>Host Agent for Windows</h3>
            <p>Windows 10 or 11, 64-bit. Setup also fetches the desktop disk, about 1.3 GB.</p>
            <button type="button" className="btn btn-share" onClick={() => onDownload("windows")}>
              <PixelIcon name="download" size={18} />
              Download for Windows
            </button>
          </article>
          <article className="download" data-tone="share">
            <PixelIcon name="terminal" size={27} className="download-icon" />
            <h3>Host Agent for Linux</h3>
            <p>Ubuntu, Debian or Arch with KVM. One script installs everything it needs.</p>
            <button type="button" className="btn btn-share" onClick={() => onDownload("linux")}>
              <PixelIcon name="download" size={18} />
              Download for Linux
            </button>
          </article>
          <article className="download" data-tone="rent">
            <PixelIcon name="globe" size={27} className="download-icon" />
            <h3>Nothing to install to use one</h3>
            <p>Standard opens in your browser. Pro streams through Moonlight, a free app.</p>
            <a className="btn btn-outline" href={routeHref(signedIn ? "sessions" : "signup")}>
              {signedIn ? "Open dashboard" : "Create an account"}
            </a>
          </article>
        </div>
      </section>

      <section className="band" id="faq">
        <div data-reveal className="band-intro">
          <h2>Good to know.</h2>
        </div>
        <div data-reveal className="faq">
          {FAQ.map((item) => (
            <details key={item.q}>
              <summary>{item.q}</summary>
              <p>{item.a}</p>
            </details>
          ))}
        </div>
      </section>

      <footer className="footer">
        <Brand />
        <p>Borrow a powerful computer. Lend the one you have.</p>
        <nav aria-label="Footer">
          <a href={routeHref("login")}>Log in</a>
          <ScrollLink to="downloads">Downloads</ScrollLink>
          <ScrollLink to="faq">FAQ</ScrollLink>
        </nav>
      </footer>
    </div>
  );
}

function ScrollLink({ to, className, children }: { to: string; className?: string; children: ReactNode }) {
  return (
    <a
      className={className}
      href={`#/`}
      onClick={(event) => {
        event.preventDefault();
        const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
        document.getElementById(to)?.scrollIntoView({ behavior: reduce ? "auto" : "smooth" });
      }}
    >
      {children}
    </a>
  );
}
