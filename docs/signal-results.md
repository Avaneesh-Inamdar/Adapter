# Signal milestone — 2026-10-22

An unchanged Mach-O consumer that installs signal dispositions, edits masks and
raises signals on itself now runs through Adapter with Darwin semantics. Delivery
is synchronous: the handler runs on the guest's current stack inside `raise` or
`kill`, or inside the `sigprocmask` call that unblocks a pending signal.

## Implementation

- `signal` and `sigaction` record a disposition per Darwin signal number (1..31)
  and return the previous one. `SIGKILL` and `SIGSTOP` cannot be changed:
  `EINVAL`. `signal` returns `SIG_ERR` on failure.
- `sigprocmask` and `pthread_sigmask` implement `SIG_BLOCK`, `SIG_UNBLOCK` and
  `SIG_SETMASK` on a single process-wide mask; `SIGKILL`/`SIGSTOP` are never
  blockable. An unknown `how` is `EINVAL`.
- A blocked signal becomes pending and is delivered when an unblock opens the
  mask, lowest signal number first.
- While a handler runs, its own signal and the registered `sa_mask` are blocked;
  the previous mask is restored on return.
- `raise` and `kill` accept signal 0 (existence check). `kill` addresses the
  guest's own pid (or 0); another pid is `ESRCH`, `-1` is `EPERM`.
- Default actions: terminate with exit status `128 + signal` and a diagnostic on
  stderr (`SIGABRT` keeps its existing "guest raised SIGABRT" message and status
  134). `SIGCHLD`, `SIGURG`, `SIGWINCH`, `SIGINFO` and `SIGIO`-class defaults are
  ignored; stop/continue signals are recorded and ignored.
- `sigemptyset`, `sigfillset`, `sigaddset`, `sigdelset` and `sigismember` use
  Darwin's 32-bit `sigset_t`.
- `sigpending`, `sigwait` (blocks until another guest thread raises one of the
  blocked signals), `sighold`, `sigrelse`, `sigignore`, `killpg` on the guest's own
  group, `bsd_signal`, the legacy `sigblock`/`sigsetmask`, `siginterrupt` (validates
  only) and `psignal` with Darwin's signal descriptions.

This module replaces the older `signal`/`sigaction`/`raise`/`sigemptyset`/
`sigaddset` stubs in `src/darwin/host/system.c`, which recorded handlers but never
ran them. `sigaltstack` remains a recorded no-op there.

## Consumers and assertions

`tests/fixtures/signals.c` asserts set algebra, `sigaction` old/new round trips,
handler execution and argument, ignored signals, block-then-pending delivery,
`kill(getpid())`, `sa_mask` blocking during a handler with the mask restored
afterwards, and `EINVAL` for `SIGKILL`, signal 0 and out-of-range members.
Expected output: `signals:dispositions, masks, raise and pending passed`.

`tests/fixtures/signals_default.c` raises an unhandled `SIGUSR1` and expects exit
status 158 with the program's later output never produced.

Both run in classic and chained form (the default-action fixture runs once).

## Boundaries

- No asynchronous delivery: Windows never interrupts guest threads, so signals
  raised by other threads, timers (`alarm`, `setitimer`) or `Ctrl-C` are not
  delivered.
- One process-wide mask, not per-thread; `pthread_kill`, `sigsuspend`, `sigpause`,
  `sigset` and `sigvec` are absent.
- `sa_flags` are not stored (`sigaction` reports 0), `SA_SIGINFO` handlers receive
  null `siginfo`/`ucontext`, and there is no `SA_RESETHAND`, `SA_NODEFER`,
  `SA_RESTART` or alternate stack use.
- Hardware faults (SIGSEGV, SIGFPE, SIGILL) are not mapped from Windows
  exceptions to guest handlers.
- No reference-Mac comparison exists for this interface.

## Reproduction

```powershell
python tests/build_fixtures.py
python -m unittest tests.integration.test_runtime -k signal
python tools/verify.py
```
