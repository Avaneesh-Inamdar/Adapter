#include "bridge.h"
#include <stdlib.h>
#include <string.h>
#include <errno.h>
#include <io.h>
#include <fcntl.h>
#include <sys/stat.h>
#include <time.h>
#include <windows.h>
#include <math.h>

static char **environment;
static SRWLOCK environment_lock = SRWLOCK_INIT;
static char *utf8(const wchar_t *s) {
    int n = WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, s, -1, NULL, 0, NULL, NULL);
    if (!n) {
        db_set_errno(92);
        return NULL;
    }
    char *p = malloc((size_t)n);
    if (!p) {
        db_set_errno(12);
        return NULL;
    }
    WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, s, -1, p, n, NULL, NULL);
    return p;
}
static void win_error(void) {
    DWORD e = GetLastError();
    int d;
    switch (e) {
    case ERROR_FILE_NOT_FOUND:
    case ERROR_PATH_NOT_FOUND:
        d = 2;
        break;
    case ERROR_ACCESS_DENIED:
    case ERROR_SHARING_VIOLATION:
        d = 13;
        break;
    case ERROR_ALREADY_EXISTS:
    case ERROR_FILE_EXISTS:
        d = 17;
        break;
    case ERROR_INVALID_HANDLE:
        d = 9;
        break;
    case ERROR_DIRECTORY:
        d = 20;
        break;
    case ERROR_FILENAME_EXCED_RANGE:
        d = 63;
        break;
    default:
        d = 5;
        break;
    }
    db_set_errno(d);
}
static void snapshot_environment(void) {
    wchar_t *block = GetEnvironmentStringsW();
    if (!block)
        return;
    size_t count = 0;
    for (wchar_t *p = block; *p; p += wcslen(p) + 1)
        if (*p != '=')
            count++;
    char **next = calloc(count + 1, sizeof(char *));
    if (!next) {
        FreeEnvironmentStringsW(block);
        return;
    }
    size_t i = 0;
    for (wchar_t *p = block; *p; p += wcslen(p) + 1)
        if (*p != '=')
            next[i++] = utf8(p);
    FreeEnvironmentStringsW(block);
    if (environment) {
        for (i = 0; environment[i]; i++)
            free(environment[i]);
        free(environment);
    }
    environment = next;
}
void db_system_begin(void) { snapshot_environment(); }
G char *db_getenv(const char *name) {
    size_t n = strlen(name);
    if (!n || strchr(name, '='))
        return NULL;
    char *value = NULL;
    AcquireSRWLockShared(&environment_lock);
    if (environment)
        for (size_t i = 0; environment[i]; i++)
            if (!strncmp(environment[i], name, n) && environment[i][n] == '=') {
                value = environment[i] + n + 1;
                break;
            }
    ReleaseSRWLockShared(&environment_lock);
    return value;
}
G int db_setenv(const char *name, const char *value, int overwrite) {
    if (!*name || strchr(name, '=')) {
        db_set_errno(22);
        return -1;
    }
    size_t len = strlen(name), count = 0, index = SIZE_MAX;
    AcquireSRWLockExclusive(&environment_lock);
    while (environment && environment[count]) {
        if (!strncmp(environment[count], name, len) && environment[count][len] == '=')
            index = count;
        count++;
    }
    if (index != SIZE_MAX && !overwrite) {
        ReleaseSRWLockExclusive(&environment_lock);
        return 0;
    }
    size_t vn = strlen(value);
    char *entry = malloc(len + vn + 2);
    char **next = calloc(count + 2, sizeof(char *));
    if (!entry || !next) {
        free(entry);
        free(next);
        ReleaseSRWLockExclusive(&environment_lock);
        db_set_errno(12);
        return -1;
    }
    memcpy(entry, name, len);
    entry[len] = '=';
    memcpy(entry + len + 1, value, vn + 1);
    if (count)
        memcpy(next, environment, count * sizeof(char *));
    if (index == SIZE_MAX)
        next[count] = entry;
    else {
        free(next[index]);
        next[index] = entry;
    }
    free(environment);
    environment = next;
    /* Only the time backend consults the CRT environment. Guest variables live
     * in an independent POSIX, case-sensitive UTF-8 environment (empty != absent). */
    if (!strcmp(name, "TZ"))
        _putenv_s("TZ", value);
    ReleaseSRWLockExclusive(&environment_lock);
    return 0;
}
G int db_unsetenv(const char *name) {
    if (!*name || strchr(name, '=')) {
        db_set_errno(22);
        return -1;
    }
    size_t n = strlen(name);
    AcquireSRWLockExclusive(&environment_lock);
    if (environment)
        for (size_t i = 0; environment[i]; i++)
            if (!strncmp(environment[i], name, n) && environment[i][n] == '=') {
                free(environment[i]);
                do {
                    environment[i] = environment[i + 1];
                    i++;
                } while (environment[i]);
                break;
            }
    if (!strcmp(name, "TZ"))
        _putenv_s("TZ", "");
    ReleaseSRWLockExclusive(&environment_lock);
    return 0;
}

typedef struct {
    int64_t sec, nsec;
} DbTimespec;
typedef struct {
    int32_t dev;
    uint16_t mode, nlink;
    uint64_t ino;
    uint32_t uid, gid;
    int32_t rdev;
    DbTimespec atime, mtime, ctime, birthtime;
    int64_t size, blocks;
    int32_t blksize;
    uint32_t flags, gen;
    int32_t spare;
    int64_t qspare[2];
} DbStat;
_Static_assert(sizeof(DbStat) == 144, "Darwin stat64");
_Static_assert(offsetof(DbStat, mode) == 4, "Darwin stat64 mode");
_Static_assert(offsetof(DbStat, size) == 96, "Darwin stat64 size");
static DbTimespec filetime(FILETIME t) {
    uint64_t ticks = ((uint64_t)t.dwHighDateTime << 32) | t.dwLowDateTime;
    int64_t unix_ticks = (int64_t)ticks - 116444736000000000LL;
    DbTimespec result = {unix_ticks / 10000000, (unix_ticks % 10000000) * 100};
    if (result.nsec < 0) {
        result.sec--;
        result.nsec += 1000000000;
    }
    return result;
}
static int stat_handle(HANDLE h, DbStat *out) {
    memset(out, 0, sizeof(*out));
    DWORD kind = GetFileType(h);
    out->blksize = 4096;
    out->nlink = 1;
    if (kind == FILE_TYPE_CHAR) {
        out->mode = 0x2000 | 0666;
        return 0;
    }
    if (kind == FILE_TYPE_PIPE) {
        out->mode = 0x1000 | 0666;
        return 0;
    }
    BY_HANDLE_FILE_INFORMATION info;
    if (!GetFileInformationByHandle(h, &info)) {
        win_error();
        return -1;
    }
    out->dev = (int32_t)info.dwVolumeSerialNumber;
    out->ino = ((uint64_t)info.nFileIndexHigh << 32) | info.nFileIndexLow;
    out->mode = (info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY ? 0x4000 | 0777 : 0x8000 | 0666);
    if (info.dwFileAttributes & FILE_ATTRIBUTE_READONLY)
        out->mode &= ~0222;
    out->nlink = (uint16_t)info.nNumberOfLinks;
    out->size = ((int64_t)info.nFileSizeHigh << 32) | info.nFileSizeLow;
    out->blocks = out->size / 512 + (out->size % 512 != 0);
    out->atime = filetime(info.ftLastAccessTime);
    out->mtime = filetime(info.ftLastWriteTime);
    out->ctime = out->mtime;
    out->birthtime = filetime(info.ftCreationTime);
    return 0;
}
static G int db_fstat(int fd, DbStat *out) {
    if (fd < 0) {
        db_set_errno(9);
        return -1;
    }
    intptr_t h = _get_osfhandle(fd);
    if (h == -1) {
        db_set_errno(9);
        return -1;
    }
    return stat_handle((HANDLE)h, out);
}
static G int db_stat(const char *path, DbStat *out) {
    wchar_t *p = db_path(path);
    if (!p)
        return -1;
    HANDLE h =
        CreateFileW(p, FILE_READ_ATTRIBUTES, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                    NULL, OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS, NULL);
    free(p);
    if (h == INVALID_HANDLE_VALUE) {
        win_error();
        return -1;
    }
    int result = stat_handle(h, out);
    CloseHandle(h);
    return result;
}
static G int db_open(const char *path, int flags, int mode) {
    /* Darwin flags, translated rather than passed through to _wopen. */
    if (flags & ~(3 | 8 | 0x200 | 0x400 | 0x800 | 0x1000000)) {
        db_set_errno(45);
        return -1;
    }
    if ((flags & 3) == 3) {
        db_set_errno(22);
        return -1;
    }
    int f = _O_BINARY | (flags & 3);
    if (flags & 8)
        f |= _O_APPEND;
    if (flags & 0x200)
        f |= _O_CREAT;
    if (flags & 0x400)
        f |= _O_TRUNC;
    if (flags & 0x800)
        f |= _O_EXCL;
    if (flags & 0x1000000)
        f |= _O_NOINHERIT;
    wchar_t *p = db_path(path);
    if (!p)
        return -1;
    int result =
        _wopen(p, f, (flags & 0x200) ? ((mode & 0200) ? _S_IREAD | _S_IWRITE : _S_IREAD) : 0);
    if (result < 0) {
        DWORD attributes = GetFileAttributesW(p);
        if (attributes != INVALID_FILE_ATTRIBUTES && (attributes & FILE_ATTRIBUTE_DIRECTORY))
            db_set_errno(21);
        else
            db_host_error();
    }
    free(p);
    return result;
}
static G int db_close(int fd) {
    int v = _close(fd);
    if (v)
        db_host_error();
    return v;
}
static G int db_isatty(int fd) {
    int v = _isatty(fd);
    if (!v)
        db_set_errno(25);
    return v;
}
static G int64_t db_pathconf(const char *path, int name) {
    DbStat s;
    if (db_stat(path, &s))
        return -1;
    if (name == 5)
        return 32767;
    if (name == 4)
        return 255;
    db_set_errno(22);
    return -1;
}
static G char *db_realpath(const char *path, char *out) {
    wchar_t *p = db_path(path);
    if (!p)
        return NULL;
    HANDLE h =
        CreateFileW(p, FILE_READ_ATTRIBUTES, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                    NULL, OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS, NULL);
    free(p);
    if (h == INVALID_HANDLE_VALUE) {
        win_error();
        return NULL;
    }
    DWORD n = GetFinalPathNameByHandleW(h, NULL, 0, FILE_NAME_NORMALIZED);
    wchar_t *full = malloc(((size_t)n + 1) * 2);
    if (!full) {
        CloseHandle(h);
        db_set_errno(12);
        return NULL;
    }
    if (!n || !GetFinalPathNameByHandleW(h, full, n + 1, FILE_NAME_NORMALIZED)) {
        win_error();
        CloseHandle(h);
        free(full);
        return NULL;
    }
    CloseHandle(h);
    wchar_t *start = full;
    if (!wcsncmp(full, L"\\\\?\\", 4)) {
        if (!wcsncmp(full + 4, L"UNC\\", 4)) {
            full[6] = '\\';
            start = full + 6;
        } else
            start += 4;
    }
    char *value = utf8(start);
    free(full);
    if (!value)
        return NULL;
    for (char *q = value; *q; q++)
        if (*q == '\\')
            *q = '/';
    if (value[0] && value[1] == ':' && value[2] == '/') {
        char drive = value[0];
        value[0] = '/';
        value[1] = drive;
    }
    if (!out)
        return value;
    if (strlen(value) >= 1024) {
        free(value);
        db_set_errno(63);
        return NULL;
    }
    strcpy(out, value);
    free(value);
    return out;
}
static int slash(char c) { return c == '/' || c == '\\'; }
static G char *db_dirname(char *path) {
    static char dot[] = ".";
    if (!path || !*path)
        return dot;
    size_t n = strlen(path);
    while (n > 1 && slash(path[n - 1]))
        path[--n] = 0;
    while (n > 0 && !slash(path[n - 1]))
        n--;
    if (!n)
        return dot;
    while (n > 1 && slash(path[n - 1]))
        n--;
    path[n] = 0;
    return path;
}
static G int db_raise(int signal) {
    /* Darwin signal numbers do not match the CRT. No guest signal handlers. */
    if (signal == 0)
        return 0;
    if (signal == 6) {
        fprintf(stderr, "adapter-run [signal]: guest raised SIGABRT\n");
        ExitProcess(134);
    }
    db_set_errno(45);
    return -1;
}

typedef struct {
    int sec, min, hour, mday, mon, year, wday, yday, isdst;
    int64_t gmtoff;
    const char *zone;
} DbTm;
_Static_assert(sizeof(DbTm) == 56, "Darwin tm");
static int64_t floor_div(int64_t a, int64_t b) {
    int64_t q = a / b;
    return q - (a % b < 0);
}
static int leap(int y) { return !(y % 4) && (y % 100 || !(y % 400)); }
/* Proleptic Gregorian civil/epoch conversion, with normalized signed fields. */
static int64_t days(int64_t y, unsigned m, unsigned d) {
    y -= m <= 2;
    int64_t era = floor_div(y, 400);
    unsigned yo = (unsigned)(y - era * 400);
    unsigned doy = (153 * (m + (m > 2 ? -3 : 9)) + 2) / 5 + d - 1;
    return era * 146097 + (int64_t)(yo * 365 + yo / 4 - yo / 100 + doy) - 719468;
}
static DbTm *utc(int64_t t, DbTm *out) {
    int64_t z = t / 86400, rem = t % 86400;
    if (rem < 0) {
        z--;
        rem += 86400;
    }
    out->hour = (int)(rem / 3600);
    out->min = (int)(rem / 60 % 60);
    out->sec = (int)(rem % 60);
    int w = (int)((z + 4) % 7);
    out->wday = w < 0 ? w + 7 : w;
    int64_t civil = z + 719468, era = floor_div(civil, 146097);
    unsigned doe = (unsigned)(civil - era * 146097);
    unsigned yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    int64_t y = (int64_t)yoe + era * 400;
    unsigned doy = doe - (365 * yoe + yoe / 4 - yoe / 100), mp = (5 * doy + 2) / 153;
    unsigned d = doy - (153 * mp + 2) / 5 + 1, m = mp + (mp < 10 ? 3 : -9);
    y += m <= 2;
    if (y - 1900 < INT32_MIN || y - 1900 > INT32_MAX) {
        db_set_errno(84);
        return NULL;
    }
    out->year = (int)y - 1900;
    out->mon = (int)m - 1;
    out->mday = (int)d;
    out->yday = (int)(z - days(y, 1, 1));
    out->isdst = 0;
    out->gmtoff = 0;
    out->zone = "UTC";
    return out;
}
static G DbTm *db_gmtime(const int64_t *t, DbTm *out) { return utc(*t, out); }
static struct tm host_tm(const DbTm *t) {
    struct tm h = {0};
    h.tm_sec = t->sec;
    h.tm_min = t->min;
    h.tm_hour = t->hour;
    h.tm_mday = t->mday;
    h.tm_mon = t->mon;
    h.tm_year = t->year;
    h.tm_wday = t->wday;
    h.tm_yday = t->yday;
    h.tm_isdst = t->isdst;
    return h;
}
static void guest_tm(const struct tm *h, DbTm *t) {
    t->sec = h->tm_sec;
    t->min = h->tm_min;
    t->hour = h->tm_hour;
    t->mday = h->tm_mday;
    t->mon = h->tm_mon;
    t->year = h->tm_year;
    t->wday = h->tm_wday;
    t->yday = h->tm_yday;
    t->isdst = h->tm_isdst;
}
static G int64_t db_timegm(DbTm *t) {
    int64_t y = (int64_t)t->year + 1900, mon = t->mon;
    y += floor_div(mon, 12);
    mon -= floor_div(mon, 12) * 12;
    int64_t value = (days(y, (unsigned)mon + 1, 1) + t->mday - 1) * 86400 +
                    (int64_t)t->hour * 3600 + (int64_t)t->min * 60 + t->sec;
    DbTm normalized;
    if (!utc(value, &normalized))
        return -1;
    *t = normalized;
    return value;
}
static G DbTm *db_localtime(const int64_t *t, DbTm *out) {
    _tzset();
    struct tm h;
    if (_localtime64_s(&h, t)) {
        db_set_errno(84);
        return NULL;
    }
    guest_tm(&h, out);
    DbTm copy = *out;
    out->gmtoff = db_timegm(&copy) - *t;
    out->zone = out->gmtoff == 0 ? "UTC" : "";
    return out;
}
static G int64_t db_mktime(DbTm *t) {
    _tzset();
    struct tm h = host_tm(t);
    int64_t result = _mktime64(&h);
    if (result == -1) {
        db_set_errno(84);
        return -1;
    }
    db_localtime(&result, t);
    return result;
}
static G int64_t db_time(int64_t *p) {
    int64_t value = _time64(NULL);
    if (p)
        *p = value;
    return value;
}
static G int db_gettimeofday(DbTimespec *out, void *tz) {
    if (tz) {
        db_set_errno(45);
        return -1;
    }
    FILETIME ft;
    GetSystemTimePreciseAsFileTime(&ft);
    DbTimespec t = filetime(ft);
    if (out) {
        out->sec = t.sec;
        out->nsec = t.nsec / 1000;
    }
    return 0;
}
static G size_t db_strftime(char *out, size_t size, const char *fmt, const DbTm *t) {
    size_t total = 0;
    struct tm h = host_tm(t);
    while (*fmt) {
        char piece[128];
        size_t n = 0;
        if (*fmt != '%') {
            piece[0] = *fmt++;
            n = 1;
        } else {
            fmt++;
            char c = *fmt++;
            if (!c)
                return 0;
            if (c == 's') {
                DbTm copy = *t;
                n = (size_t)snprintf(piece, sizeof(piece), "%lld", (long long)db_timegm(&copy));
            } else if (c == 'Z') {
                n = strlen(t->zone ? t->zone : "");
                if (n >= sizeof(piece))
                    return 0;
                memcpy(piece, t->zone ? t->zone : "", n);
            } else if (c == 'z') {
                int64_t off = t->gmtoff;
                char sign = off < 0 ? '-' : '+';
                if (off < 0)
                    off = -off;
                n = (size_t)snprintf(piece, sizeof(piece), "%c%02lld%02lld", sign,
                                     (long long)(off / 3600), (long long)(off / 60 % 60));
            } else if (c == 'e')
                n = (size_t)snprintf(piece, sizeof(piece), "%2d", t->mday);
            else if (c == 'u')
                n = (size_t)snprintf(piece, sizeof(piece), "%d", t->wday ? t->wday : 7);
            else {
                const char *expanded = NULL;
                char spec[3] = {'%', c, 0};
                if (c == 'F')
                    expanded = "%Y-%m-%d";
                else if (c == 'T')
                    expanded = "%H:%M:%S";
                else if (c == 'R')
                    expanded = "%H:%M";
                else if (c == 'D')
                    expanded = "%m/%d/%y";
                if (!expanded && !strchr("aAbBcdHIjmMpSUwWxXyY%nt", c)) {
                    db_set_errno(45);
                    return 0;
                }
                if (c == 'n') {
                    piece[0] = '\n';
                    n = 1;
                } else if (c == 't') {
                    piece[0] = '\t';
                    n = 1;
                } else
                    n = strftime(piece, sizeof(piece), expanded ? expanded : spec, &h);
                if (!n)
                    return 0;
            }
        }
        if (total >= size || n >= size - total) {
            if (size)
                out[0] = 0;
            return 0;
        }
        memcpy(out + total, piece, n);
        total += n;
    }
    if (total >= size)
        return 0;
    out[total] = 0;
    return total;
}
static int number(const char **p, int min, int max, int digits, int *out) {
    const char *s = *p;
    int value = 0, count = 0;
    while (count < digits && *s >= '0' && *s <= '9') {
        value = value * 10 + *s++ - '0';
        count++;
    }
    if (!count || value < min || value > max)
        return 0;
    *p = s;
    *out = value;
    return 1;
}
static const char *parse_time(const char *s, const char *fmt, DbTm *t) {
    while (*fmt) {
        char c = *fmt++;
        if (c == ' ' || c == '\n' || c == '\t') {
            while (*s == ' ' || *s == '\n' || *s == '\t' || *s == '\r')
                s++;
            continue;
        }
        if (c != '%') {
            if (*s++ != c)
                return NULL;
            continue;
        }
        c = *fmt++;
        int v;
        switch (c) {
        case '%':
            if (*s++ != '%')
                return NULL;
            break;
        case 'Y':
            if (!number(&s, 0, 9999, 4, &v))
                return NULL;
            t->year = v - 1900;
            break;
        case 'y':
            if (!number(&s, 0, 99, 2, &v))
                return NULL;
            t->year = v + (v <= 68 ? 100 : 0);
            break;
        case 'm':
            if (!number(&s, 1, 12, 2, &v))
                return NULL;
            t->mon = v - 1;
            break;
        case 'e':
            while (*s == ' ')
                s++; /* fall through */
        case 'd':
            if (!number(&s, 1, 31, 2, &v))
                return NULL;
            t->mday = v;
            break;
        case 'H':
            if (!number(&s, 0, 23, 2, &v))
                return NULL;
            t->hour = v;
            break;
        case 'M':
            if (!number(&s, 0, 59, 2, &v))
                return NULL;
            t->min = v;
            break;
        case 'S':
            if (!number(&s, 0, 60, 2, &v))
                return NULL;
            t->sec = v;
            break;
        case 'j':
            if (!number(&s, 1, 366, 3, &v))
                return NULL;
            t->yday = v - 1;
            break;
        case 'w':
            if (!number(&s, 0, 6, 1, &v))
                return NULL;
            t->wday = v;
            break;
        case 'F':
            s = parse_time(s, "%Y-%m-%d", t);
            if (!s)
                return NULL;
            break;
        case 'T':
            s = parse_time(s, "%H:%M:%S", t);
            if (!s)
                return NULL;
            break;
        case 'R':
            s = parse_time(s, "%H:%M", t);
            if (!s)
                return NULL;
            break;
        case 'D':
            s = parse_time(s, "%m/%d/%y", t);
            if (!s)
                return NULL;
            break;
        case 'n':
        case 't':
            while (*s == ' ' || *s == '\n' || *s == '\t' || *s == '\r')
                s++;
            break;
        default:
            db_set_errno(45);
            return NULL;
        }
    }
    return s;
}
static G char *db_strptime(const char *s, const char *fmt, DbTm *t) {
    const char *end = parse_time(s, fmt, t);
    if (!end)
        return NULL;
    if (t->mon >= 0 && t->mon < 12 && t->mday > 0 && t->mday <= 31) {
        int64_t date = days((int64_t)t->year + 1900, (unsigned)t->mon + 1, (unsigned)t->mday);
        t->wday = (int)((date + 4) % 7);
        if (t->wday < 0)
            t->wday += 7;
        t->yday = (int)(date - days((int64_t)t->year + 1900, 1, 1));
        if (t->yday == 365 && !leap(t->year + 1900))
            return NULL;
    }
    return (char *)end;
}
double db_nexttoward_entry(double x, double y, int direction) {
    if (isnan(x) || isnan(y))
        return x + y;
    if (!direction)
        return y;
    return nextafter(x, direction > 0 ? INFINITY : -INFINITY);
}
G double db_nexttoward(double x, ...);
uintptr_t db_system_symbol(const char *name) {
    LOOKUP("_environ", &environment);
    LOOKUP("_getenv", db_getenv);
    LOOKUP("_setenv", db_setenv);
    LOOKUP("_unsetenv", db_unsetenv);
    LOOKUP("_stat$INODE64", db_stat);
    LOOKUP("_fstat$INODE64", db_fstat);
    LOOKUP("_open", db_open);
    LOOKUP("_close", db_close);
    LOOKUP("_isatty", db_isatty);
    LOOKUP("_realpath$DARWIN_EXTSN", db_realpath);
    LOOKUP("_dirname", db_dirname);
    LOOKUP("_pathconf", db_pathconf);
    LOOKUP("_raise", db_raise);
    LOOKUP("_gmtime_r", db_gmtime);
    LOOKUP("_localtime_r", db_localtime);
    LOOKUP("_timegm", db_timegm);
    LOOKUP("_mktime", db_mktime);
    LOOKUP("_time", db_time);
    LOOKUP("_gettimeofday", db_gettimeofday);
    LOOKUP("_strftime", db_strftime);
    LOOKUP("_strptime", db_strptime);
    LOOKUP("_nexttoward", db_nexttoward);
    return 0;
}
