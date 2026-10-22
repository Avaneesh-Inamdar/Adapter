/* sysctl and libproc consumer: names, MIB paths, size queries, short buffers,
   read-only enforcement, cross-checks against uname and the process itself. */
typedef unsigned long size_t;
typedef long time_t;

struct timeval {
    long tv_sec;
    int tv_usec;
    int pad;
};

struct utsname {
    char sysname[256];
    char nodename[256];
    char release[256];
    char version[256];
    char machine[256];
};

extern int puts(const char *);
extern int *__error(void);
extern int getpid(void);
extern time_t time(time_t *);
extern size_t strlen(const char *);
extern int strcmp(const char *, const char *);
extern int uname(struct utsname *);
extern int sysctl(int *mib, unsigned int length, void *old, size_t *old_length, void *new_value, size_t new_length);
extern int sysctlbyname(const char *name, void *old, size_t *old_length, void *new_value, size_t new_length);
extern int sysctlnametomib(const char *name, int *mib, size_t *size);
extern int proc_libversion(int *major, int *minor);
extern int proc_name(int pid, void *buffer, unsigned int size);
extern int proc_pidpath(int pid, void *buffer, unsigned int size);
extern int proc_listpids(unsigned int type, unsigned int info, void *buffer, int size);
extern int proc_pidinfo(int pid, int flavor, unsigned long long argument, void *buffer, int size);

#define ENOENT 2
#define EPERM 1
#define ENOMEM 12
#define EINVAL 22
#define ESRCH 3

static int failures;
static int step;
static void expect(long actual, long wanted) {
    step = step + 1;
    if (actual != wanted)
        failures = failures + 1 + step * 1000;
}

static int int_of(const char *name) {
    int value = -1;
    size_t length = sizeof(value);
    if (sysctlbyname(name, &value, &length, (void *)0, 0) != 0 || length != sizeof(value))
        return -1;
    return value;
}

static unsigned long long quad_of(const char *name) {
    unsigned long long value = 0;
    size_t length = sizeof(value);
    if (sysctlbyname(name, &value, &length, (void *)0, 0) != 0 || length != sizeof(value))
        return 0;
    return value;
}

int main(void) {
    struct utsname host;
    char text[256];
    size_t length;
    int mib[4];
    size_t mib_size;

    /* Strings agree with uname. */
    expect(uname(&host), 0);
    length = sizeof(text);
    expect(sysctlbyname("kern.ostype", text, &length, (void *)0, 0), 0);
    expect(strcmp(text, "Darwin"), 0);
    expect((long)length, 7);
    length = sizeof(text);
    expect(sysctlbyname("kern.osrelease", text, &length, (void *)0, 0), 0);
    expect(strcmp(text, host.release), 0);
    length = sizeof(text);
    expect(sysctlbyname("kern.version", text, &length, (void *)0, 0), 0);
    expect(strcmp(text, host.version), 0);
    length = sizeof(text);
    expect(sysctlbyname("hw.machine", text, &length, (void *)0, 0), 0);
    expect(strcmp(text, host.machine), 0);

    /* A NULL buffer asks for the size; a short buffer gets a prefix and ENOMEM. */
    length = 0;
    expect(sysctlbyname("kern.version", (void *)0, &length, (void *)0, 0), 0);
    expect((long)length, (long)strlen(host.version) + 1);
    {
        char small[4] = {1, 1, 1, 1};
        length = sizeof(small);
        expect(sysctlbyname("kern.version", small, &length, (void *)0, 0), -1);
        expect(*__error(), ENOMEM);
        expect((long)length, (long)strlen(host.version) + 1);
        expect(small[0], host.version[0]);
        expect(small[3], host.version[3]);
    }

    /* Counts, sizes and types are consistent with one another. */
    expect(int_of("hw.ncpu") >= 1, 1);
    expect(int_of("hw.logicalcpu"), int_of("hw.ncpu"));
    expect(int_of("hw.activecpu"), int_of("hw.ncpu"));
    expect(int_of("hw.physicalcpu") >= 1 && int_of("hw.physicalcpu") <= int_of("hw.logicalcpu"), 1);
    expect(int_of("hw.pagesize"), 4096);
    expect(int_of("hw.byteorder"), 1234);
    expect(quad_of("hw.memsize") > (1ULL << 30), 1);
    expect(int_of("hw.physmem") > 0, 1);
    expect(quad_of("hw.cachelinesize") > 0, 1);
    expect(quad_of("hw.tbfrequency"), 1000000000);
    expect(int_of("hw.optional.x86_64"), 1);
    expect(int_of("hw.cpu64bit_capable"), 1);
    length = sizeof(text);
    expect(sysctlbyname("machdep.cpu.brand_string", text, &length, (void *)0, 0), 0);
    expect(strlen(text) > 0, 1);
    expect(int_of("kern.argmax"), 1048576);
    expect(int_of("user.posix2_version"), 200112);
    expect(int_of("user.bc_dim_max"), 2048);
    expect(int_of("user.re_dup_max"), 255);
    expect(int_of("hw.optional.floatingpoint"), 1);
    expect(int_of("kern.maxproc") > 0, 1);

    /* The boot time is a struct timeval in the past. */
    {
        struct timeval boot;
        length = sizeof(boot);
        expect(sysctlbyname("kern.boottime", &boot, &length, (void *)0, 0), 0);
        expect((long)length, 16);
        expect(boot.tv_sec > 1000000000 && boot.tv_sec <= time((time_t *)0), 1);
    }

    /* MIB spellings reach the same values, including the name2oid call libc uses. */
    mib_size = 4;
    expect(sysctlnametomib("hw.ncpu", mib, &mib_size), 0);
    expect((long)mib_size, 2);
    expect(mib[0], 6);
    expect(mib[1], 3);
    {
        int value = -1;
        length = sizeof(value);
        expect(sysctl(mib, 2, &value, &length, (void *)0, 0), 0);
        expect(value, int_of("hw.ncpu"));
    }
    mib[0] = 1;
    mib[1] = 65;
    length = sizeof(text);
    expect(sysctl(mib, 2, text, &length, (void *)0, 0), 0);
    {
        char build[32];
        size_t build_length = sizeof(build);
        expect(sysctlbyname("kern.osversion", build, &build_length, (void *)0, 0), 0);
        expect(strcmp(text, build), 0);
    }
    {
        int oid[4] = {0, 3, 0, 0};
        int resolved[4] = {0, 0, 0, 0};
        size_t resolved_length = sizeof(resolved);
        expect(sysctl(oid, 2, resolved, &resolved_length, "hw.pagesize", 11), 0);
        expect((long)resolved_length, 8);
        expect(resolved[0], 6);
        expect(resolved[1], 7);
    }
    mib_size = 1;
    expect(sysctlnametomib("hw.ncpu", mib, &mib_size), -1);
    expect(*__error(), ENOMEM);

    /* Unknown names, writes and bad arguments fail with Darwin's errors. */
    length = sizeof(text);
    expect(sysctlbyname("no.such.name", text, &length, (void *)0, 0), -1);
    expect(*__error(), ENOENT);
    {
        int value = 3;
        length = sizeof(value);
        expect(sysctlbyname("hw.ncpu", &value, &length, &value, sizeof(value)), -1);
        expect(*__error(), EPERM);
    }
    expect(sysctlbyname((const char *)0, text, &length, (void *)0, 0), -1);
    expect(*__error(), EINVAL);
    mib[0] = 6;
    expect(sysctl(mib, 1, text, &length, (void *)0, 0), -1);
    expect(*__error(), EINVAL);
    mib[0] = 99;
    mib[1] = 99;
    expect(sysctl(mib, 2, text, &length, (void *)0, 0), -1);
    expect(*__error(), ENOENT);

    /* libproc describes the guest's own process only. */
    {
        int pid = getpid();
        char name[64];
        char path[512];
        int pids[4] = {0, 0, 0, 0};
        int major = 0;
        int minor = 0;
        unsigned long long task[12];
        unsigned char bsd[136];
        expect(proc_libversion(&major, &minor), 0);
        expect(major, 1);
        expect(proc_name(pid, name, sizeof(name)) > 0, 1);
        expect(name[0] == 's' && name[1] == 'y' && name[2] == 's', 1);
        expect(proc_pidpath(pid, path, sizeof(path)) > 0, 1);
        expect((long)strlen(path) >= (long)strlen(name), 1);
        expect(proc_name(pid + 1, name, sizeof(name)), 0);
        expect(*__error(), ESRCH);
        expect(proc_listpids(1, 0, (void *)0, 0), 4);
        expect(proc_listpids(1, 0, pids, sizeof(pids)), 4);
        expect(pids[0], pid);
        expect(proc_pidinfo(pid, 4, 0, task, sizeof(task)), 96);
        expect(task[1] > 0, 1);
        expect(proc_pidinfo(pid, 3, 0, bsd, sizeof(bsd)), 136);
        expect(*(unsigned int *)(bsd + 12), (unsigned int)pid);
        expect(bsd[48] != 0, 1);
        expect(proc_pidinfo(pid, 3, 0, bsd, 8), 0);
        expect(*__error(), EINVAL);
        expect(proc_pidinfo(pid, 99, 0, bsd, sizeof(bsd)), 0);
        expect(*__error(), EINVAL);
        expect(proc_pidinfo(pid + 1, 3, 0, bsd, sizeof(bsd)), 0);
        expect(*__error(), ESRCH);
    }

    if (failures != 0) {
        char report[] = "sysctl:contract failed (code 0000000)";
        int at = 35;
        int code = failures;
        while (code > 0 && at >= 29) {
            report[at] = (char)('0' + code % 10);
            code = code / 10;
            at = at - 1;
        }
        puts(report);
        return 1;
    }
    puts("sysctl:names, MIB paths, size rules, errors and libproc passed");
    return 0;
}
