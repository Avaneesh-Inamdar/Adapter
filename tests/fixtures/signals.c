/* Darwin signal consumer: dispositions, masks, pending delivery and errors. */
typedef unsigned int u32;

struct sigaction {
    void *handler;
    u32 mask;
    int flags;
};

extern int puts(const char *);
extern int *__error(void);
extern void *signal(int sig, void *handler);
extern int sigaction(int sig, const struct sigaction *act, struct sigaction *old);
extern int raise(int sig);
extern int kill(int pid, int sig);
extern int getpid(void);
extern int sigprocmask(int how, const u32 *set, u32 *old);
extern int sigemptyset(u32 *set);
extern int sigfillset(u32 *set);
extern int sigaddset(u32 *set, int sig);
extern int sigdelset(u32 *set, int sig);
extern int sigismember(const u32 *set, int sig);
extern int sigpending(u32 *set);
extern int sigwait(const u32 *set, int *sig);
extern int sighold(int sig);
extern int sigrelse(int sig);
extern int sigignore(int sig);
extern int siginterrupt(int sig, int flag);
extern int killpg(int group, int sig);
extern int sigblock(int mask);
extern int sigsetmask(int mask);
extern void psignal(int sig, const char *message);
extern void *bsd_signal(int sig, void *handler);

#define SIGKILL 9
#define SIGUSR1 30
#define SIGUSR2 31
#define SIG_DFL ((void *)0)
#define SIG_IGN ((void *)1)
#define EINVAL 22
#define BLOCK 1
#define UNBLOCK 2
#define SETMASK 3
#define BIT(sig) (1u << ((sig) - 1))

static int hits;
static int last_signal;

static void on_usr1(int sig) {
    hits = hits + 1;
    last_signal = sig;
}

static u32 seen_mask;

static void on_usr2(int sig) {
    (void)sig;
    sigprocmask(BLOCK, (const u32 *)0, &seen_mask);
}

static int failures;

static void expect(int step, int actual, int wanted) {
    if (actual != wanted) {
        failures = failures + 1 + step * 1000;
    }
}

int main(void) {
    u32 set = 0;
    struct sigaction act;
    struct sigaction old;
    int step = 0;

    expect(step++, sigemptyset(&set), 0);
    expect(step++, set, 0);
    expect(step++, sigaddset(&set, SIGUSR1), 0);
    expect(step++, sigismember(&set, SIGUSR1), 1);
    expect(step++, sigismember(&set, SIGUSR2), 0);
    expect(step++, sigdelset(&set, SIGUSR1), 0);
    expect(step++, sigismember(&set, SIGUSR1), 0);
    expect(step++, sigfillset(&set), 0);
    expect(step++, sigismember(&set, SIGKILL), 1);

    act.handler = (void *)&on_usr1;
    act.mask = 0;
    act.flags = 0;
    old.handler = (void *)8;
    old.mask = 7;
    old.flags = 1;
    expect(step++, sigaction(SIGUSR1, &act, &old), 0);
    expect(step++, old.handler == SIG_DFL, 1);
    expect(step++, old.mask, 0);
    expect(step++, raise(SIGUSR1), 0);
    expect(step++, hits, 1);
    expect(step++, last_signal, SIGUSR1);
    expect(step++, sigaction(SIGUSR1, (const struct sigaction *)0, &old), 0);
    expect(step++, old.handler == (void *)&on_usr1, 1);

    /* Ignored signals run nothing and do not terminate the guest. */
    expect(step++, signal(SIGUSR2, SIG_IGN) == SIG_DFL, 1);
    expect(step++, raise(SIGUSR2), 0);
    expect(step++, hits, 1);
    expect(step++, signal(SIGUSR2, SIG_DFL) == SIG_IGN, 1);

    /* A blocked signal stays pending until the mask opens. */
    expect(step++, sigprocmask(BLOCK, &set, (u32 *)0) == 0, 1);
    expect(step++, raise(SIGUSR1), 0);
    expect(step++, hits, 1);
    expect(step++, sigemptyset(&set), 0);
    expect(step++, sigaddset(&set, SIGUSR1), 0);
    expect(step++, sigprocmask(UNBLOCK, &set, (u32 *)0), 0);
    expect(step++, hits, 2);

    /* The calling process may address itself; other pids are reported. */
    expect(step++, kill(getpid(), SIGUSR1), 0);
    expect(step++, hits, 3);

    /* While a handler runs its own signal and its sa_mask are blocked, and the
     * previous mask returns afterwards. */
    expect(step++, sigemptyset(&set), 0);
    expect(step++, sigprocmask(SETMASK, &set, (u32 *)0), 0);
    act.handler = (void *)&on_usr2;
    act.mask = BIT(SIGUSR1);
    expect(step++, sigaction(SIGUSR2, &act, (struct sigaction *)0), 0);
    expect(step++, raise(SIGUSR2), 0);
    expect(step++, seen_mask, BIT(SIGUSR1) | BIT(SIGUSR2));
    expect(step++, sigprocmask(BLOCK, (const u32 *)0, &set), 0);
    expect(step++, set, 0);
    act.mask = 0;
    act.handler = (void *)&on_usr1;

    /* Hold/release, pending sets and sigwait consume a blocked signal without running it. */
    {
        u32 pending = 1;
        int got = 0;
        expect(step++, sighold(SIGUSR1), 0);
        expect(step++, raise(SIGUSR1), 0);
        expect(step++, hits, 3);
        expect(step++, sigpending(&pending), 0);
        expect(step++, pending, BIT(SIGUSR1));
        expect(step++, sigwait(&pending, &got), 0);
        expect(step++, got, SIGUSR1);
        expect(step++, sigpending(&pending), 0);
        expect(step++, pending, 0);
        expect(step++, sigrelse(SIGUSR1), 0);
        expect(step++, hits, 3);

        /* killpg addresses the own group; other groups are unknown. */
        expect(step++, killpg(getpid(), SIGUSR1), 0);
        expect(step++, hits, 4);
        expect(step++, killpg(getpid() + 1, SIGUSR1), -1);
        expect(step++, *__error(), 3);

        /* The legacy int-mask calls report the previous mask. */
        expect(step++, sigblock((int)BIT(SIGUSR2)), 0);
        expect(step++, sigsetmask(0), (int)BIT(SIGUSR2));
        expect(step++, siginterrupt(SIGUSR1, 1), 0);
        expect(step++, siginterrupt(0, 1), -1);
        expect(step++, bsd_signal(SIGUSR2, SIG_IGN) == (void *)&on_usr2, 1);
        expect(step++, raise(SIGUSR2), 0);
        expect(step++, sigignore(SIGUSR1), 0);
        expect(step++, raise(SIGUSR1), 0);
        expect(step++, hits, 4);
        expect(step++, sigignore(SIGKILL), -1);
        act.handler = (void *)&on_usr1;
        expect(step++, sigaction(SIGUSR1, &act, (struct sigaction *)0), 0);
        psignal(SIGUSR1, "signals-fixture");
    }

    /* A disposition that Darwin forbids changes nothing and reports EINVAL. */
    expect(step++, sigaction(SIGKILL, &act, &old), -1);
    expect(step++, *__error(), EINVAL);
    expect(step++, raise(0), -1);
    expect(step++, *__error(), EINVAL);
    expect(step++, sigismember(&set, 0), -1);
    expect(step++, *__error(), EINVAL);

    expect(step++, sigaction(SIGUSR1, (const struct sigaction *)0, &old), 0);
    act.handler = SIG_DFL;
    expect(step++, sigaction(SIGUSR1, &act, (struct sigaction *)0), 0);

    if (failures != 0) {
        char text[] = "signals:disposition contract failed (code 0000000)";
        int at = 48;
        int code = failures;
        while (code > 0 && at >= 42) {
            text[at] = (char)('0' + code % 10);
            code = code / 10;
            at = at - 1;
        }
        puts(text);
        return 1;
    }
    puts("signals:dispositions, masks, raise and pending passed");
    return 0;
}
