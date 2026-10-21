typedef unsigned long size_t;
typedef struct File File;
typedef struct {
    long sec, nsec;
} Timespec;
typedef struct {
    int dev;
    unsigned short mode, nlink;
    unsigned long ino;
    unsigned uid, gid;
    int rdev;
    Timespec atime, mtime, ctime, birth;
    long size, blocks;
    int blksize;
    unsigned flags, gen;
    int spare;
    long qspare[2];
} Stat;
typedef struct {
    int sec, min, hour, mday, mon, year, wday, yday, isdst;
    long gmtoff;
    char *zone;
} Tm;
extern File *fopen(const char *, const char *) __asm__("_fopen$DARWIN_EXTSN");
extern File *fdopen(int, const char *) __asm__("_fdopen$DARWIN_EXTSN");
extern int stat(const char *, Stat *) __asm__("_stat$INODE64");
extern int fstat(int, Stat *) __asm__("_fstat$INODE64");
extern char *realpath(const char *, char *) __asm__("_realpath$DARWIN_EXTSN");
extern int open(const char *, int, ...), close(int), fileno(File *), fclose(File *), feof(File *),
    ferror(File *), fflush(File *), getc(File *), puts(const char *),
    strcmp(const char *, const char *), memcmp(const void *, const void *, size_t), *__error(void);
extern size_t fwrite(const void *, size_t, size_t, File *), fread(void *, size_t, size_t, File *),
    strlen(const char *), strftime(char *, size_t, const char *, const Tm *);
extern char *fgets(char *, int, File *), *strptime(const char *, const char *, Tm *),
    *dirname(char *);
extern long timegm(Tm *), time(long *), pathconf(const char *, int);
extern Tm *gmtime_r(const long *, Tm *);
extern int gettimeofday(Timespec *, void *);
extern void free(void *);
#define CHECK(e)                                                                                   \
    do {                                                                                           \
        if (!(e))                                                                                  \
            return __LINE__;                                                                       \
    } while (0)
int main(int argc, char **argv) {
    CHECK(argc == 2 && sizeof(Stat) == 144 && sizeof(Tm) == 56);
    CHECK(!fopen(argv[1], "") && *__error() == 22);
    const char bytes[] = "one\r\ntwo\n\0\xff";
    File *f = fopen(argv[1], "w+");
    CHECK(f);
    CHECK(fwrite(bytes, 2, 6, f) == 6 && fflush(f) == 0);
    struct {
        unsigned long before;
        Stat s;
        unsigned long after;
    } guarded = {.before = 0x123456789abcdef0UL, .after = 0xfedcba9876543210UL};
    CHECK(fstat(fileno(f), &guarded.s) == 0 && guarded.s.size == 12 &&
          (guarded.s.mode & 0xf000) == 0x8000);
    CHECK(guarded.before == 0x123456789abcdef0UL && guarded.after == 0xfedcba9876543210UL);
    CHECK(fclose(f) == 0 && stat(argv[1], &guarded.s) == 0 && guarded.s.size == 12);
    f = fopen(argv[1], "r");
    CHECK(f);
    char data[32];
    CHECK(fgets(data, 32, f) && !strcmp(data, "one\r\n"));
    CHECK(fread(data, 1, 32, f) == 7 && !memcmp(data, "two\n\0\xff", 7) && feof(f) && !ferror(f));
    CHECK(fclose(f) == 0);
    int fd = open(argv[1], 0);
    CHECK(fd >= 0);
    f = fdopen(fd, "r");
    CHECK(f && getc(f) == 'o' && fclose(f) == 0);
    char *full = realpath(argv[1], 0);
    CHECK(full && full[0] == '/' && full[2] == '/' && pathconf(full, 5) >= 1024);
    free(full);
    char name[] = "/C/one/two/";
    CHECK(!strcmp(dirname(name), "/C/one"));
    char missing[1024];
    size_t n = strlen(argv[1]);
    CHECK(n < 1000);
    for (size_t i = 0; i < n; i++)
        missing[i] = argv[1][i];
    missing[n] = '!';
    missing[n + 1] = 0;
    CHECK(!fopen(missing, "r") && *__error() == 2);
    long epoch = -86400;
    Tm t;
    CHECK(gmtime_r(&epoch, &t) == &t && t.year == 69 && t.mon == 11 && t.mday == 31 &&
          t.wday == 3 && t.yday == 364);
    CHECK(timegm(&t) == epoch && strftime(data, sizeof(data), "%FT%TZ", &t) == 20 &&
          !strcmp(data, "1969-12-31T00:00:00Z"));
    Tm parsed = {.wday = 8, .yday = 367};
    char *end = strptime("2000-02-29T12:34:56Ztail", "%Y-%m-%dT%H:%M:%SZ", &parsed);
    CHECK(end && !strcmp(end, "tail") && parsed.year == 100 && parsed.mon == 1 &&
          parsed.mday == 29 && parsed.yday == 59 && parsed.wday == 2);
    CHECK(timegm(&parsed) == 951827696);
    parsed.mon = 12;
    parsed.mday = 1;
    parsed.hour = 0;
    parsed.min = 0;
    parsed.sec = 0;
    CHECK(timegm(&parsed) == 978307200 && parsed.year == 101 && parsed.mon == 0);
    Timespec now;
    long seconds;
    CHECK(gettimeofday(&now, 0) == 0 && now.nsec >= 0 && now.nsec < 1000000 && time(&seconds) > 0);
    CHECK(seconds >= now.sec && seconds - now.sec <= 2);
    puts("libc:UTF-8 files, stat64, errno and UTC time passed");
    return 0;
}
