/* Default disposition: an unhandled terminating signal ends the guest. */
extern int puts(const char *);
extern int raise(int sig);

#define SIGUSR1 30

int main(void) {
    puts("signals_default:before");
    raise(SIGUSR1);
    puts("signals_default:unreachable");
    return 0;
}
