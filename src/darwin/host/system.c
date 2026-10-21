#include "bridge.h"
#include <stdlib.h>
#include <string.h>
#include <errno.h>
#include <io.h>
#include <process.h>
#include <direct.h>
#include <fcntl.h>
#include <sys/stat.h>
#include <time.h>
#include <windows.h>
#include <bcrypt.h>
#include <math.h>

static char **environment;
static int guest_argc;
static char **guest_argv;
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
static void set_descriptor_flags(int fd, int flags);
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
    } else
        set_descriptor_flags(result, flags & ~(0x200 | 0x400 | 0x800 | 0x1000000));
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
static void guest_path(char *value) {
    for (char *q = value; *q; q++)
        if (*q == '\\')
            *q = '/';
    if (value[0] && value[1] == ':' && value[2] == '/') {
        char drive = value[0];
        value[0] = '/';
        value[1] = drive;
    }
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
    guest_path(value);
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

/* Signal handlers are recorded faithfully so probing code sees success, but
 * Windows never delivers POSIX signals to this adapter, so stored handlers are
 * never invoked. sigaltstack is recorded for the same reason. */
typedef struct {
    uint64_t handler;
    uint32_t mask;
    int32_t flags;
} DbSigaction;
typedef struct {
    void *sp;
    size_t size;
    int32_t flags;
    int32_t pad;
} DbStack;
static DbSigaction signal_actions[32];
static DbStack alternate_stack;
static SRWLOCK signal_lock = SRWLOCK_INIT;

static G int db_sigaction(int sig, const DbSigaction *act, DbSigaction *old) {
    if (sig <= 0 || sig >= 32 || sig == 9 || sig == 17) {
        db_set_errno(22);
        return -1;
    }
    AcquireSRWLockExclusive(&signal_lock);
    if (old)
        *old = signal_actions[sig];
    if (act)
        signal_actions[sig] = *act;
    ReleaseSRWLockExclusive(&signal_lock);
    return 0;
}
static G int db_sigemptyset(uint32_t *set) {
    if (!set) {
        db_set_errno(14);
        return -1;
    }
    *set = 0;
    return 0;
}
static G int db_sigaddset(uint32_t *set, int sig) {
    if (!set || sig <= 0 || sig >= 32) {
        db_set_errno(22);
        return -1;
    }
    *set |= 1u << (sig - 1);
    return 0;
}
static G int db_sigaltstack(const DbStack *ss, DbStack *old) {
    AcquireSRWLockExclusive(&signal_lock);
    if (old)
        *old = alternate_stack;
    if (ss)
        alternate_stack = *ss;
    ReleaseSRWLockExclusive(&signal_lock);
    return 0;
}
static G uint64_t db_signal(int sig, uint64_t handler) {
    if (sig <= 0 || sig >= 32 || sig == 9 || sig == 17) {
        db_set_errno(22);
        return UINT64_MAX;
    }
    AcquireSRWLockExclusive(&signal_lock);
    uint64_t old = signal_actions[sig].handler;
    signal_actions[sig].handler = handler;
    signal_actions[sig].mask = 0;
    signal_actions[sig].flags = 0;
    ReleaseSRWLockExclusive(&signal_lock);
    return old;
}
static G int db_pause(void) {
    for (;;)
        Sleep(INFINITE);
}

/* Single-user identity: guests see process id and the root (0) uid/gid, and
 * identity changes succeed without persisting, matching root privileges. */
static G int db_getpid(void) { return _getpid(); }
static G uint32_t db_getuid(void) { return 0; }
static G uint32_t db_geteuid(void) { return 0; }
static G int db_setuid(uint32_t uid) {
    (void)uid;
    return 0;
}
static G int db_setgid(uint32_t gid) {
    (void)gid;
    return 0;
}
static G int db_seteuid(uint32_t uid) {
    (void)uid;
    return 0;
}
static G int db_setegid(uint32_t gid) {
    (void)gid;
    return 0;
}
/* Child processes are a separate guest program with its own mapped image and
 * address space; the runtime deliberately supports one active guest per host
 * process, so fork/exec/spawn report ENOSYS rather than faking a child. */
static G int db_fork(void) {
    db_set_errno(78);
    return -1;
}
static G int db_execvp(const char *file, char *const argv[]) {
    (void)file;
    (void)argv;
    db_set_errno(78);
    return -1;
}
static G int db_waitpid(int pid, int *status, int options) {
    (void)pid;
    (void)status;
    (void)options;
    db_set_errno(10); /* no child can ever have been created */
    return -1;
}
static G int db_setpgid(int pid, int group) {
    (void)pid;
    (void)group;
    db_set_errno(78);
    return -1;
}
static G int db_setsid(void) {
    db_set_errno(78);
    return -1;
}
static G int db_setgroups(int count, const uint32_t *groups) {
    (void)count;
    (void)groups;
    db_set_errno(78);
    return -1;
}
#define SPAWN_ACTIONS_MAGIC 0x53464131u
#define SPAWN_ATTR_MAGIC 0x53415431u
typedef struct {
    uint32_t magic;
} DbSpawnActions;
typedef struct {
    uint32_t magic;
    int32_t flags;
    int32_t pgroup;
    uint32_t sigdefault;
} DbSpawnAttr;
static G int db_spawn_file_actions_init(void **actions) {
    if (!actions)
        return 22;
    DbSpawnActions *value = calloc(1, sizeof(*value));
    if (!value)
        return 12;
    value->magic = SPAWN_ACTIONS_MAGIC;
    *actions = value;
    return 0;
}
static G int db_spawn_file_actions_destroy(void **actions) {
    if (!actions || !*actions || ((DbSpawnActions *)*actions)->magic != SPAWN_ACTIONS_MAGIC)
        return 22;
    free(*actions);
    *actions = NULL;
    return 0;
}
static G int db_spawn_file_actions_adddup2(void **actions, int fd, int newfd) {
    if (!actions || !*actions || ((DbSpawnActions *)*actions)->magic != SPAWN_ACTIONS_MAGIC ||
        fd < 0 || newfd < 0)
        return 22;
    return 0;
}
static G int db_spawn_attr_init(void **attributes) {
    if (!attributes)
        return 22;
    DbSpawnAttr *value = calloc(1, sizeof(*value));
    if (!value)
        return 12;
    value->magic = SPAWN_ATTR_MAGIC;
    *attributes = value;
    return 0;
}
static G int db_spawn_attr_destroy(void **attributes) {
    if (!attributes || !*attributes || ((DbSpawnAttr *)*attributes)->magic != SPAWN_ATTR_MAGIC)
        return 22;
    free(*attributes);
    *attributes = NULL;
    return 0;
}
static G int db_spawn_attr_setflags(void **attributes, int16_t flags) {
    if (!attributes || !*attributes || ((DbSpawnAttr *)*attributes)->magic != SPAWN_ATTR_MAGIC)
        return 22;
    ((DbSpawnAttr *)*attributes)->flags = flags;
    return 0;
}
static G int db_spawn_attr_setpgroup(void **attributes, int pgroup) {
    if (!attributes || !*attributes || ((DbSpawnAttr *)*attributes)->magic != SPAWN_ATTR_MAGIC)
        return 22;
    ((DbSpawnAttr *)*attributes)->pgroup = pgroup;
    return 0;
}
static G int db_spawn_attr_setsigdefault(void **attributes, const uint32_t *set) {
    if (!attributes || !*attributes || ((DbSpawnAttr *)*attributes)->magic != SPAWN_ATTR_MAGIC)
        return 22;
    ((DbSpawnAttr *)*attributes)->sigdefault = set ? *set : 0;
    return 0;
}
static G int db_posix_spawnp(int *pid, const char *file, const void **actions,
                             const void **attributes, char *const argv[], char *const envp[]) {
    (void)file;
    (void)actions;
    (void)attributes;
    (void)argv;
    (void)envp;
    (void)pid;
    return 78; /* posix_spawn reports the error as its return value */
}
/* Aggregation only plausibly changes NTFS ownership, so reject it like a
 * filesystem without chattr rights rather than silently pretending. */
static int ownership_ok(const char *path, uint32_t uid, uint32_t gid) {
    DbStat s;
    if (db_stat(path, &s))
        return 0;
    if ((uid == (uint32_t)-1 || uid == 0) && (gid == (uint32_t)-1 || gid == 0))
        return 1;
    db_set_errno(1); /* EPERM */
    return 0;
}
static G int db_chown(const char *path, uint32_t uid, uint32_t gid) {
    return ownership_ok(path, uid, gid) ? 0 : -1;
}
static G int db_lchown(const char *path, uint32_t uid, uint32_t gid) {
    return ownership_ok(path, uid, gid) ? 0 : -1;
}

/* File checks: macOS test bits (F_OK 0, X_OK 1, W_OK 2, R_OK 4). X_OK is
 * answered as R_OK because guest executable rights are not kept on NTFS. */
static G int db_access(const char *path, int mode) {
    if (mode & ~7) {
        db_set_errno(22);
        return -1;
    }
    wchar_t *p = db_path(path);
    if (!p)
        return -1;
    DWORD attributes = GetFileAttributesW(p);
    free(p);
    if (attributes == INVALID_FILE_ATTRIBUTES) {
        win_error();
        return -1;
    }
    if ((mode & 2) && (attributes & FILE_ATTRIBUTE_READONLY)) {
        db_set_errno(13);
        return -1;
    }
    return 0;
}
/* 0200 in the guest mode does not clear, but NTFS only knows a readonly bit. */
static G int db_chmod(const char *path, unsigned mode) {
    wchar_t *p = db_path(path);
    if (!p)
        return -1;
    DWORD attributes = GetFileAttributesW(p);
    if (attributes == INVALID_FILE_ATTRIBUTES) {
        free(p);
        win_error();
        return -1;
    }
    DWORD target = attributes & ~FILE_ATTRIBUTE_READONLY;
    if (!(mode & 0200))
        target |= FILE_ATTRIBUTE_READONLY;
    if (!SetFileAttributesW(p, target)) {
        free(p);
        win_error();
        return -1;
    }
    free(p);
    return 0;
}
static G int db_mkdir(const char *path, unsigned mode) {
    (void)mode; /* POSIX mode bits are not representable beyond readonly. */
    wchar_t *p = db_path(path);
    if (!p)
        return -1;
    int result = _wmkdir(p);
    if (result)
        win_error();
    free(p);
    return result;
}
static G int db_unlink(const char *path) {
    wchar_t *p = db_path(path);
    if (!p)
        return -1;
    DWORD attributes = GetFileAttributesW(p);
    if (attributes != INVALID_FILE_ATTRIBUTES && (attributes & FILE_ATTRIBUTE_DIRECTORY)) {
        free(p);
        db_set_errno(1); /* POSIX unlink(2) cannot remove directories. */
        return -1;
    }
    int result = _wunlink(p) ? -1 : 0;
    if (result)
        win_error();
    free(p);
    return result;
}
static G int db_rmdir(const char *path) {
    wchar_t *p = db_path(path);
    if (!p)
        return -1;
    DWORD attributes = GetFileAttributesW(p);
    if (attributes != INVALID_FILE_ATTRIBUTES && !(attributes & FILE_ATTRIBUTE_DIRECTORY)) {
        free(p);
        db_set_errno(20); /* ENOTDIR */
        return -1;
    }
    int result = _wrmdir(p) ? -1 : 0;
    if (result)
        win_error();
    free(p);
    return result;
}
static G int db_rename(const char *old_path, const char *new_path) {
    wchar_t *from = db_path(old_path), *to = db_path(new_path);
    if (!from || !to) {
        free(from);
        free(to);
        return -1;
    }
    DWORD source = GetFileAttributesW(from), target = GetFileAttributesW(to);
    if (target != INVALID_FILE_ATTRIBUTES && (target & FILE_ATTRIBUTE_DIRECTORY) &&
        (source == INVALID_FILE_ATTRIBUTES || !(source & FILE_ATTRIBUTE_DIRECTORY))) {
        free(from);
        free(to);
        db_set_errno(21); /* EISDIR: a file cannot replace a directory. */
        return -1;
    }
    if (target != INVALID_FILE_ATTRIBUTES && (target & FILE_ATTRIBUTE_READONLY))
        SetFileAttributesW(to, target & ~FILE_ATTRIBUTE_READONLY);
    int result = _wrename(from, to) ? -1 : 0;
    if (result) {
        if (errno == EXDEV)
            db_set_errno(18); /* Darwin EXDEV value, not the CRT one. */
        else if (target != INVALID_FILE_ATTRIBUTES && (target & FILE_ATTRIBUTE_DIRECTORY))
            db_set_errno(66); /* ENOTEMPTY: directory replacement is unsupported. */
        else
            db_host_error();
    }
    free(from);
    free(to);
    return result;
}
static G int64_t db_write(int fd, const void *buffer, size_t count) {
    if (count > INT_MAX) {
        db_set_errno(84);
        return -1;
    }
    int n = _write(fd, buffer, (unsigned)count);
    if (n < 0) {
        db_host_error();
        return -1;
    }
    return n;
}
static G unsigned db_sleep(unsigned seconds) {
    Sleep(seconds > 4294967u ? 4294967295u : seconds * 1000u);
    return 0; /* No guest signals can interrupt a sleep. */
}
static G int db_usleep(uint32_t useconds) {
    Sleep(useconds / 1000 + (useconds % 1000 ? 1 : 0));
    return 0;
}

/* BSD random(3), transcribed from Apple Libc stdlib/FreeBSD/random.c so guest
 * sequences keep their deterministic VALUES, not merely plausible noise. */
#define DEG_3 31
#define SEP_3 3
#define TYPE_3 3
#define NSHUFF 50
static uint32_t randtbl[DEG_3 + 1] = {
    TYPE_3,     0x991539b1, 0x16a5bce3, 0x6774a4cd, 0x3e01511e, 0x4e508aaa, 0x61048c05, 0xf5500617,
    0x846b7115, 0x6a19892c, 0x896a97af, 0xdb48f936, 0x14898454, 0x37ffd106, 0xb58bff9c, 0x59e17104,
    0xcf918a49, 0x09378c83, 0x52c7a471, 0x8d293ea9, 0x1f4fc301, 0xc3db71be, 0x39b44e1c, 0xf8a44ef9,
    0x4c8b80b1, 0x19edc328, 0x87bf4bdd, 0xc9b240e5, 0xe9ee4b1b, 0x4382aee7, 0x535b6b41, 0xf3bec5da};
static uint32_t *rand_front = &randtbl[SEP_3 + 1];
static uint32_t *rand_rear = &randtbl[1];
static uint32_t *rand_state = &randtbl[1];
static int rand_deg = DEG_3;
static int rand_sep = SEP_3;
static uint32_t *rand_end = &randtbl[DEG_3 + 1];
static uint32_t good_rand(int32_t x) {
    /* x = (7^5 * x) mod (2^31 - 1), Park and Miller (CACM 31.10.1988). */
    int32_t hi, lo;
    if (x == 0)
        x = 123459876;
    hi = x / 127773;
    lo = x % 127773;
    x = 16807 * lo - 2836 * hi;
    if (x < 0)
        x += 0x7fffffff;
    return (uint32_t)x;
}
static uint32_t advance_random(void) {
    uint32_t i;
    uint32_t *f = rand_front, *r = rand_rear;
    *f += *r;
    i = (*f >> 1) & 0x7fffffff; /* Discard the least random bit. */
    if (++f >= rand_end) {
        f = rand_state;
        ++r;
    } else if (++r >= rand_end) {
        r = rand_state;
    }
    rand_front = f;
    rand_rear = r;
    return i;
}
static void seed_random(uint32_t x) {
    int i, lim;
    rand_state[0] = x;
    for (i = 1; i < rand_deg; i++)
        rand_state[i] = good_rand((int32_t)rand_state[i - 1]);
    rand_front = &rand_state[rand_sep];
    rand_rear = &rand_state[0];
    lim = 10 * rand_deg; /* NSHUFF applies only to the trivial TYPE_0 path. */
    for (i = 0; i < lim; i++)
        (void)advance_random();
}
static G void db_srandom(uint32_t x) { seed_random(x); }
static G void db_srandomdev(void) {
    size_t len = (size_t)rand_deg * sizeof(uint32_t);
    if (BCryptGenRandom(NULL, (unsigned char *)rand_state, (unsigned long)len,
                        BCRYPT_USE_SYSTEM_PREFERRED_RNG) >= 0) {
        rand_front = &rand_state[rand_sep];
        rand_rear = &rand_state[0];
        return;
    }
    FILETIME now;
    GetSystemTimePreciseAsFileTime(&now);
    DbTimespec tv = filetime(now);
    seed_random((uint32_t)(((uint32_t)_getpid() << 16) ^ (uint32_t)tv.sec ^ (uint32_t)tv.nsec));
}
static G int64_t db_random(void) { return (int64_t)advance_random(); }

typedef G int (*DbCompare)(const void *, const void *);
static G void *db_bsearch(const void *key, const void *base, size_t count, size_t size,
                          DbCompare compare) {
    while (count > 0) {
        const void *probe = (const char *)base + (count / 2) * size;
        int order = compare(key, probe);
        if (order == 0)
            return (void *)probe;
        if (order < 0)
            count /= 2;
        else {
            base = (const char *)probe + size;
            count -= count / 2 + 1;
        }
    }
    return NULL;
}
static G int64_t db_ftell(DbFile *f) {
    if (!f || !f->host) {
        db_set_errno(9);
        return -1;
    }
    int64_t v = _ftelli64(f->host);
    if (v < 0)
        db_host_error();
    return v;
}
static G uint32_t db_arc4random_uniform(uint32_t bound) {
    /* OpenBSD arc4random_uniform: rejection sampling avoids modulo bias. */
    if (bound < 2)
        return 0;
    uint32_t floor = (uint32_t)(0u - bound) % bound;
    for (;;) {
        uint32_t value;
        if (BCryptGenRandom(NULL, (unsigned char *)&value, 4, BCRYPT_USE_SYSTEM_PREFERRED_RNG) <
            0) {
            fprintf(stderr, "adapter-run [random]: entropy source unavailable\n");
            ExitProcess(134);
        }
        if (value >= floor)
            return value % bound;
    }
}
static const char mkdtemp_alphabet[] = "ABCDEFGHIJKLMNOPQRSTUVWXYZ"
                                       "abcdefghijklmnopqrstuvwxyz"
                                       "0123456789";
static G char *db_mkdtemp(char *tmpl) {
    size_t n = strlen(tmpl);
    if (n < 6) {
        db_set_errno(22);
        return NULL;
    }
    size_t xs = 0;
    while (xs < n && tmpl[n - 1 - xs] == 'X')
        xs++;
    if (xs < 6) {
        db_set_errno(22); /* POSIX requires at least six trailing X placeholders. */
        return NULL;
    }
    for (int attempt = 0; attempt < 1000; attempt++) {
        for (size_t i = 0; i < 6; i++) {
            unsigned char pick;
            if (BCryptGenRandom(NULL, &pick, 1, BCRYPT_USE_SYSTEM_PREFERRED_RNG) < 0)
                return NULL;
            tmpl[n - 6 + i] = mkdtemp_alphabet[pick % 62];
        }
        wchar_t *p = db_path(tmpl);
        if (!p)
            return NULL;
        if (_wmkdir(p) == 0) {
            free(p);
            return tmpl;
        }
        free(p);
        if (errno != EEXIST) {
            db_host_error();
            return NULL;
        }
    }
    db_set_errno(17); /* EEXIST after the POSIX retry budget is exhausted. */
    return NULL;
}
static G int db_gethostname(char *name, int length) {
    if (!name || length <= 0) {
        db_set_errno(22);
        return -1;
    }
    wchar_t value[256];
    DWORD size = 256;
    if (!GetComputerNameExW(ComputerNameDnsHostname, value, &size)) {
        win_error();
        return -1;
    }
    char *utf8_name = utf8(value);
    if (!utf8_name)
        return -1;
    size_t n = strlen(utf8_name);
    if ((int)n + 1 > length) {
        free(utf8_name);
        db_set_errno(22); /* FreeBSD gethostname(3): EINVAL when too short. */
        return -1;
    }
    memcpy(name, utf8_name, n + 1);
    free(utf8_name);
    return 0;
}
static G unsigned db_umask(unsigned mode) {
    /* The CRT umask only distinguishes read/write, but the caller-visible guest
     * value keeps all POSIX bits. */
    static unsigned previous = 022;
    int crt = 0;
    if (mode & 0400)
        crt |= _S_IREAD;
    if (mode & 0200)
        crt |= _S_IWRITE;
    (void)_umask(crt);
    unsigned old = previous;
    previous = mode & 0777;
    return old;
}
typedef struct {
    int64_t sec;
    int32_t usec;
} DbTimeval;
_Static_assert(sizeof(DbTimeval) == 16, "Darwin timeval LP64");
static void filetime_of(const DbTimeval *t, FILETIME *out) {
    uint64_t ticks = (uint64_t)((*t).sec + 11644473600) * 10000000 + (uint64_t)t->usec * 10;
    out->dwLowDateTime = (DWORD)ticks;
    out->dwHighDateTime = (DWORD)(ticks >> 32);
}
static G int db_utimes(const char *path, const DbTimeval *times) {
    wchar_t *p = db_path(path);
    if (!p)
        return -1;
    HANDLE h = CreateFileW(p, FILE_WRITE_ATTRIBUTES,
                           FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, NULL,
                           OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS, NULL);
    free(p);
    if (h == INVALID_HANDLE_VALUE) {
        win_error();
        return -1;
    }
    FILETIME now, access, modified;
    GetSystemTimePreciseAsFileTime(&now);
    access = now;
    modified = now;
    if (times) {
        filetime_of(&times[0], &access);
        filetime_of(&times[1], &modified);
    }
    int result = SetFileTime(h, NULL, &access, &modified) ? 0 : -1;
    if (result)
        win_error();
    CloseHandle(h);
    return result;
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
    if (!t) {
        db_set_errno(22);
        return -1;
    }
    t->isdst = 0;
    int64_t y = (int64_t)t->year + 1900, mon = t->mon;
    y += floor_div(mon, 12);
    mon -= floor_div(mon, 12) * 12;
    int64_t value = (days(y, (unsigned)mon + 1, 1) + t->mday - 1) * 86400 +
                    (int64_t)t->hour * 3600 + (int64_t)t->min * 60 + t->sec;
    DbTm normalized;
    if (!utc(value, &normalized))
        return -1;
    /* Darwin time2sub rejects normalized calendar years below 1900 before
     * adding saved seconds. Its retry also normalizes seconds first. */
    if (normalized.year < 0) {
        DbTm before_seconds;
        if (!utc(value - t->sec, &before_seconds) || before_seconds.year < 0)
            return -1;
    }
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
static G DbTm *db_localtime1(const int64_t *t) {
    static DbTm out;
    return db_localtime(t, &out);
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
G double db_nexttoward(double x, ...);
double db_nexttoward_entry(double x, double y, int direction) {
    if (isnan(x) || isnan(y))
        return x + y;
    if (!direction)
        return y;
    return nextafter(x, direction > 0 ? INFINITY : -INFINITY);
}
static char *guest_executable_path;

/* Called once by the worker before constructors run; the guest name is the
 * same path it sees in argv[0]. The string is intentionally unbounded by
 * MAX_PATH because Darwin returns whatever execve received. */
void db_set_executable_path(const char *path) {
    char *copy = path ? _strdup(path) : NULL;
    if (!copy)
        return;
    free(guest_executable_path);
    guest_executable_path = copy;
}

static G int db_NSGetExecutablePath(char *buf, uint32_t *bufsize) {
    if (!bufsize) {
        db_set_errno(22);
        return -1;
    }
    char fallback[4096];
    const char *p = guest_executable_path;
    if (!p) {
        wchar_t wpath[MAX_PATH];
        DWORD len = GetModuleFileNameW(NULL, wpath, MAX_PATH);
        int u8len = len ? WideCharToMultiByte(CP_UTF8, 0, wpath, (int)len, fallback,
                                              sizeof(fallback) - 1, NULL, NULL)
                        : 0;
        if (u8len > 0) {
            fallback[u8len] = 0;
            p = fallback;
        } else {
            p = "";
        }
    }
    size_t needed = strlen(p) + 1;
    if (needed > UINT32_MAX) {
        db_set_errno(63);
        return -1;
    }
    if (!buf || *bufsize < (uint32_t)needed) {
        *bufsize = (uint32_t)needed;
        return -1;
    }
    memcpy(buf, p, needed);
    return 0;
}

void db_set_arguments(int argc, char **argv) {
    if (argc < 0 || (argc && !argv))
        return;
    char **copy = calloc((size_t)argc + 1, sizeof(char *));
    if (!copy)
        return;
    int i;
    for (i = 0; i < argc; i++) {
        copy[i] = argv[i] ? _strdup(argv[i]) : NULL;
        if (argv[i] && !copy[i]) {
            while (i--)
                free(copy[i]);
            free(copy);
            return;
        }
    }
    for (i = 0; i < guest_argc; i++)
        free(guest_argv[i]);
    free(guest_argv);
    guest_argc = argc;
    guest_argv = copy;
}
static G int *db_NSGetArgc(void) { return &guest_argc; }
static G char ***db_NSGetArgv(void) { return &guest_argv; }
static G char ***db_NSGetEnviron(void) { return &environment; }
static G int db_CCRandomGenerateBytes(void *bytes, size_t count) {
    if (!count)
        return 0;
    if (!bytes || count > ULONG_MAX)
        return -1;
    return BCryptGenRandom(NULL, bytes, (ULONG)count, BCRYPT_USE_SYSTEM_PREFERRED_RNG) < 0 ? -1 : 0;
}
static G int db_clock_gettime(int clock_id, DbTimespec *out) {
    if (!out) {
        db_set_errno(22);
        return -1;
    }
    switch (clock_id) {
    case 0: { /* CLOCK_REALTIME */
        FILETIME now;
        GetSystemTimePreciseAsFileTime(&now);
        *out = filetime(now);
        return 0;
    }
    case 4:   /* CLOCK_MONOTONIC_RAW */
    case 6:   /* CLOCK_MONOTONIC */
    case 8: { /* CLOCK_UPTIME_RAW */
        LARGE_INTEGER counter, frequency;
        if (!QueryPerformanceCounter(&counter) || !QueryPerformanceFrequency(&frequency)) {
            db_set_errno(22);
            return -1;
        }
        out->sec = counter.QuadPart / frequency.QuadPart;
        out->nsec =
            (int64_t)((counter.QuadPart % frequency.QuadPart) * 1000000000 / frequency.QuadPart);
        return 0;
    }
    case 12:   /* CLOCK_PROCESS_CPUTIME_ID */
    case 16: { /* CLOCK_THREAD_CPUTIME_ID */
        FILETIME created, exited, kernel, user;
        BOOL ok = clock_id == 12
                      ? GetProcessTimes(GetCurrentProcess(), &created, &exited, &kernel, &user)
                      : GetThreadTimes(GetCurrentThread(), &created, &exited, &kernel, &user);
        if (!ok) {
            win_error();
            return -1;
        }
        uint64_t kernel_ticks = ((uint64_t)kernel.dwHighDateTime << 32) | kernel.dwLowDateTime;
        uint64_t user_ticks = ((uint64_t)user.dwHighDateTime << 32) | user.dwLowDateTime;
        uint64_t ticks = kernel_ticks + user_ticks;
        out->sec = (int64_t)(ticks / 10000000);
        out->nsec = (int64_t)((ticks % 10000000) * 100);
        return 0;
    }
    default:
        db_set_errno(22);
        return -1;
    }
}
static G int db_nanosleep(const DbTimespec *request, DbTimespec *remaining) {
    if (!request || request->sec < 0 || request->nsec < 0 || request->nsec >= 1000000000) {
        db_set_errno(22);
        return -1;
    }
    uint64_t ns = (uint64_t)request->sec * 1000000000 + (uint64_t)request->nsec;
    uint64_t ms = (ns + 999999) / 1000000;
    while (ms) {
        DWORD chunk = ms > 60000 ? 60000 : (DWORD)ms;
        Sleep(chunk);
        ms -= chunk;
    }
    if (remaining) {
        remaining->sec = 0;
        remaining->nsec = 0;
    }
    return 0;
}
static G int db_sched_yield(void) {
    SwitchToThread();
    return 0;
}
static G char *db_getcwd(char *buf, size_t size) {
    DWORD needed = GetCurrentDirectoryW(0, NULL);
    if (!needed) {
        win_error();
        return NULL;
    }
    wchar_t *wide_path = malloc((size_t)needed * sizeof(wchar_t));
    if (!wide_path) {
        db_set_errno(12);
        return NULL;
    }
    if (!GetCurrentDirectoryW(needed, wide_path)) {
        free(wide_path);
        win_error();
        return NULL;
    }
    char *value = utf8(wide_path);
    free(wide_path);
    if (!value)
        return NULL;
    guest_path(value);
    size_t length = strlen(value) + 1;
    if (!buf)
        return value;
    if (size < length) {
        free(value);
        db_set_errno(34); /* ERANGE */
        return NULL;
    }
    memcpy(buf, value, length);
    free(value);
    return buf;
}
static G int db_chdir(const char *path) {
    wchar_t *p = db_path(path);
    if (!p)
        return -1;
    int result = SetCurrentDirectoryW(p) ? 0 : -1;
    if (result)
        win_error();
    free(p);
    return result;
}
static G int db_chroot(const char *path) {
    (void)path;
    db_set_errno(45); /* ENOTSUP: the host has no process-local filesystem root. */
    return -1;
}
static G int64_t db_sysconf(int name) {
    SYSTEM_INFO info;
    switch (name) {
    case 1: /* _SC_ARG_MAX */
        return 1048576;
    case 2: /* _SC_CHILD_MAX */
        return 266;
    case 3: /* _SC_CLK_TCK */
        return 100;
    case 4: /* _SC_NGROUPS_MAX */
        return 16;
    case 5: /* _SC_OPEN_MAX */
        return _getmaxstdio();
    case 6: /* _SC_JOB_CONTROL */
    case 7: /* _SC_SAVED_IDS */
        return 1;
    case 8: /* _SC_VERSION */
        return 200112;
    case 29: /* _SC_PAGESIZE / _SC_PAGE_SIZE */
        GetSystemInfo(&info);
        return info.dwPageSize;
    case 57: /* _SC_NPROCESSORS_CONF */
    case 58: /* _SC_NPROCESSORS_ONLN */
        GetSystemInfo(&info);
        return info.dwNumberOfProcessors;
    case 72: /* _SC_HOST_NAME_MAX */
    case 73: /* _SC_LOGIN_NAME_MAX */
    case 74: /* _SC_TTY_NAME_MAX */
        return 255;
    case 200: { /* _SC_PHYS_PAGES */
        MEMORYSTATUSEX memory = {.dwLength = sizeof(memory)};
        if (!GlobalMemoryStatusEx(&memory)) {
            db_set_errno(22);
            return -1;
        }
        GetSystemInfo(&info);
        return (int64_t)(memory.ullTotalPhys / info.dwPageSize);
    }
    default:
        db_set_errno(22);
        return -1;
    }
}
typedef struct {
    char sysname[256];
    char nodename[256];
    char release[256];
    char version[256];
    char machine[256];
} DbUtsname;
_Static_assert(sizeof(DbUtsname) == 1280, "Darwin utsname");
static void copy_field(char *field, size_t size, const char *value) {
    size_t length = strlen(value);
    if (length >= size)
        length = size - 1;
    memcpy(field, value, length);
    field[length] = 0;
}
static G int db_uname(DbUtsname *out) {
    if (!out) {
        db_set_errno(22);
        return -1;
    }
    char hostname[256] = "";
    if (db_gethostname(hostname, sizeof(hostname)))
        hostname[0] = 0;
    memset(out, 0, sizeof(*out));
    copy_field(out->sysname, sizeof(out->sysname), "Darwin");
    copy_field(out->nodename, sizeof(out->nodename), hostname);
    copy_field(out->release, sizeof(out->release), "24.6.0");
    copy_field(out->version, sizeof(out->version),
               "Darwin Kernel Version 24.6.0: Adapter compatibility layer");
    copy_field(out->machine, sizeof(out->machine), "x86_64");
    return 0;
}
static G int db_strerror_r(int error, char *buf, size_t size) {
    char *message = strerror(error);
    if (!buf || !size)
        return 22;
    size_t length = strlen(message);
    if (length >= size)
        return 34;
    memcpy(buf, message, length + 1);
    return 0;
}
typedef struct {
    char *pw_name;
    char *pw_passwd;
    uint32_t pw_uid, pw_gid;
    int64_t pw_change;
    char *pw_class;
    char *pw_gecos;
    char *pw_dir;
    char *pw_shell;
    int64_t pw_expire;
} DbPasswd;
_Static_assert(sizeof(DbPasswd) == 72, "Darwin passwd");
static G int db_getpwuid_r(uint32_t uid, DbPasswd *result, char *buf, size_t size,
                           DbPasswd **found) {
    if (!result || !found) {
        db_set_errno(22);
        return 22;
    }
    *found = NULL;
    if (uid != 0)
        return 0;
    const char *name = db_getenv("USERNAME");
    if (!name || !*name)
        name = "adapter";
    const char *home = db_getenv("USERPROFILE");
    if (!home || !*home)
        home = "C:";
    const char *fields[] = {name, "*", "", name, home, "/bin/sh"};
    size_t used = 0;
    for (size_t i = 0; i < sizeof(fields) / sizeof(fields[0]); i++)
        used += strlen(fields[i]) + 1;
    if (!buf || used > size)
        return 34;
    char *cursor = buf;
    char *places[6];
    for (size_t i = 0; i < sizeof(fields) / sizeof(fields[0]); i++) {
        size_t length = strlen(fields[i]) + 1;
        memcpy(cursor, fields[i], length);
        places[i] = cursor;
        cursor += length;
    }
    memset(result, 0, sizeof(*result));
    result->pw_name = places[0];
    result->pw_passwd = places[1];
    result->pw_uid = 0;
    result->pw_gid = 0;
    result->pw_class = places[2];
    result->pw_gecos = places[3];
    result->pw_dir = places[4];
    result->pw_shell = places[5];
    *found = result;
    return 0;
}
static G void *db_memmove_chk(void *dest, const void *src, size_t count, size_t dest_size) {
    if (dest_size < count)
        db_guest_exit(134);
    return memmove(dest, src, count);
}
static G void db_memset_pattern16(void *dest, const void *pattern, size_t count) {
    unsigned char *d = dest;
    const unsigned char *p = pattern;
    for (size_t done = 0; done < count;) {
        size_t chunk = count - done < 16 ? count - done : 16;
        memcpy(d + done, p, chunk);
        done += chunk;
    }
}
static G void db_immediate_exit(int code) {
    _exit(code); /* C `_exit`: no atexit handlers or stdio flush. */
}
static G int64_t db_read(int fd, void *buffer, size_t count) {
    if (!count)
        return 0;
    if (!buffer) {
        db_set_errno(14); /* EFAULT */
        return -1;
    }
    int chunk = count > INT_MAX ? INT_MAX : (int)count;
    int result = _read(fd, buffer, chunk);
    if (result < 0)
        db_host_error();
    return result;
}
typedef struct {
    void *base;
    size_t length;
} DbIovec;
static G int64_t db_writev(int fd, const DbIovec *vectors, int count) {
    if (count < 0 || count > 1024 || (count && !vectors)) {
        db_set_errno(22);
        return -1;
    }
    int64_t total = 0;
    for (int i = 0; i < count; i++) {
        if (!vectors[i].length)
            continue;
        int64_t written = db_write(fd, vectors[i].base, vectors[i].length);
        if (written < 0)
            return total ? total : -1;
        total += written;
        if ((size_t)written < vectors[i].length)
            break;
    }
    return total;
}
static G int64_t db_lseek(int fd, int64_t offset, int whence) {
    int64_t result = _lseeki64(fd, offset, whence);
    if (result < 0)
        db_host_error();
    return result;
}
static G int db_dup(int fd) {
    int result = _dup(fd);
    if (result < 0)
        db_host_error();
    return result;
}
static G int db_dup2(int old_fd, int new_fd) {
    int result = _dup2(old_fd, new_fd);
    if (result < 0)
        db_host_error();
    return result;
}
static G int db_pipe(int *fds) {
    int result = _pipe(fds, 4096, _O_BINARY);
    if (result)
        db_host_error();
    else {
        set_descriptor_flags(fds[0], 0);
        set_descriptor_flags(fds[1], 1);
    }
    return result;
}
#define DB_MAX_FDS 8192
static int descriptor_flags[DB_MAX_FDS];
static int descriptor_flags_ready;
static SRWLOCK descriptor_lock = SRWLOCK_INIT;
static void set_descriptor_flags(int fd, int flags) {
    if (fd < 0 || fd >= DB_MAX_FDS)
        return;
    AcquireSRWLockExclusive(&descriptor_lock);
    if (!descriptor_flags_ready) {
        descriptor_flags[0] = 0;
        descriptor_flags[1] = 1;
        descriptor_flags[2] = 1;
        descriptor_flags_ready = 1;
    }
    descriptor_flags[fd] = flags;
    ReleaseSRWLockExclusive(&descriptor_lock);
}
static int get_descriptor_flags(int fd) {
    if (fd < 0 || fd >= DB_MAX_FDS)
        return -1;
    AcquireSRWLockShared(&descriptor_lock);
    int flags = descriptor_flags_ready ? descriptor_flags[fd] : -1;
    ReleaseSRWLockShared(&descriptor_lock);
    return flags;
}
/* Darwin fcntl commands and status bits. The runtime owns no socket layer, so
 * O_NONBLOCK is bookkeeping; opening modes and O_APPEND remain meaningful. */
static G int db_fcntl(int fd, int command, int value) {
    switch (command) {
    case 0:  /* F_DUPFD */
    case 67: /* F_DUPFD_CLOEXEC; children are unsupported, so close-on-exec is moot */
        return db_dup(fd);
    case 1: /* F_GETFD */
        return 0;
    case 2: /* F_SETFD */
        return 0;
    case 3: { /* F_GETFL */
        int flags = get_descriptor_flags(fd);
        if (flags < 0) {
            db_set_errno(9);
            return -1;
        }
        return flags;
    }
    case 4: { /* F_SETFL */
        int flags = get_descriptor_flags(fd);
        if (flags < 0) {
            db_set_errno(9);
            return -1;
        }
        if (value & ~(0x4 | 0x8)) {
            db_set_errno(22);
            return -1;
        }
        set_descriptor_flags(fd, (flags & ~(0x4 | 0x8)) | (value & (0x4 | 0x8)));
        return 0;
    }
    default:
        db_set_errno(45); /* ENOTSUP */
        return -1;
    }
}
typedef struct {
    uint16_t rows, columns, xpixels, ypixels;
} DbWinsize;
static G int db_ioctl(int fd, unsigned long request, uintptr_t argument) {
    if (request == 0x40087468UL) { /* TIOCGWINSZ */
        if (!argument) {
            db_set_errno(14);
            return -1;
        }
        DbWinsize *size = (DbWinsize *)argument;
        intptr_t handle = _get_osfhandle(fd);
        CONSOLE_SCREEN_BUFFER_INFO info;
        if (handle != -1 && GetConsoleScreenBufferInfo((HANDLE)handle, &info)) {
            size->rows = (uint16_t)(info.srWindow.Bottom - info.srWindow.Top + 1);
            size->columns = (uint16_t)(info.srWindow.Right - info.srWindow.Left + 1);
            size->xpixels = 0;
            size->ypixels = 0;
            return 0;
        }
        if (_isatty(fd)) {
            size->rows = 24;
            size->columns = 80;
            size->xpixels = 0;
            size->ypixels = 0;
            return 0;
        }
        db_set_errno(25);
        return -1;
    }
    if (request == 0x4004667fUL) { /* FIONREAD */
        if (!argument) {
            db_set_errno(14);
            return -1;
        }
        intptr_t handle = _get_osfhandle(fd);
        DWORD available = 0;
        if (handle == -1 || !PeekNamedPipe((HANDLE)handle, NULL, 0, NULL, &available, NULL))
            available = 0;
        *(int *)argument = (int)available;
        return 0;
    }
    if (request == 0x8004667eUL) { /* FIONBIO */
        if (!argument) {
            db_set_errno(14);
            return -1;
        }
        int enable = *(int *)argument;
        int flags = get_descriptor_flags(fd);
        if (flags < 0) {
            db_set_errno(9);
            return -1;
        }
        set_descriptor_flags(fd, enable ? (flags | 0x4) : (flags & ~0x4));
        return 0;
    }
    db_set_errno(25); /* ENOTTY */
    return -1;
}
#define DB_AT_FDCWD (-2)
#define DB_DIR_MAGIC 0x4449525f41445054ULL
typedef struct {
    uint64_t d_ino;
    uint64_t d_seekoff;
    uint16_t d_reclen;
    uint16_t d_namlen;
    uint8_t d_type;
    uint8_t d_name[1024];
} DbDirent;
_Static_assert(sizeof(DbDirent) == 1048, "Darwin dirent");
_Static_assert(offsetof(DbDirent, d_name) == 21, "Darwin dirent name");
typedef struct {
    uint64_t magic;
    HANDLE find;
    int fd;
    int first;
    int done;
    wchar_t *pattern;
} DbDir;
static G int db_fstatat(int fd, const char *path, DbStat *out, int flags) {
    if (!path || !out) {
        db_set_errno(22);
        return -1;
    }
    if (flags & ~0x20) {
        db_set_errno(22);
        return -1;
    }
    if (fd == DB_AT_FDCWD || path[0] == '/')
        return db_stat(path, out);
    intptr_t raw = _get_osfhandle(fd);
    if (raw == -1) {
        db_set_errno(9);
        return -1;
    }
    DWORD length = GetFinalPathNameByHandleW((HANDLE)raw, NULL, 0, FILE_NAME_NORMALIZED);
    if (!length) {
        win_error();
        return -1;
    }
    wchar_t *base = malloc(((size_t)length + 1) * sizeof(wchar_t));
    wchar_t *relative = db_path(path);
    if (!base || !relative) {
        free(base);
        free(relative);
        db_set_errno(12);
        return -1;
    }
    if (!GetFinalPathNameByHandleW((HANDLE)raw, base, length + 1, FILE_NAME_NORMALIZED)) {
        free(base);
        free(relative);
        win_error();
        return -1;
    }
    wchar_t *start = base;
    if (!wcsncmp(start, L"\\\\?\\", 4))
        start += 4;
    size_t root_length = wcslen(start), leaf_length = wcslen(relative);
    wchar_t *combined = malloc((root_length + leaf_length + 2) * sizeof(wchar_t));
    if (!combined) {
        free(base);
        free(relative);
        db_set_errno(12);
        return -1;
    }
    memcpy(combined, start, root_length * sizeof(wchar_t));
    combined[root_length] = '\\';
    memcpy(combined + root_length + 1, relative, (leaf_length + 1) * sizeof(wchar_t));
    char *utf8_path = utf8(combined);
    free(combined);
    free(base);
    free(relative);
    if (!utf8_path)
        return -1;
    int result = db_stat(utf8_path, out);
    free(utf8_path);
    return result;
}
static G DbDir *db_opendir(const char *path) {
    if (!path) {
        db_set_errno(22);
        return NULL;
    }
    wchar_t *p = db_path(path);
    if (!p)
        return NULL;
    HANDLE handle = CreateFileW(p, FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES,
                                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, NULL,
                                OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS, NULL);
    if (handle == INVALID_HANDLE_VALUE) {
        win_error();
        free(p);
        return NULL;
    }
    int fd = _open_osfhandle((intptr_t)handle, _O_RDONLY | _O_BINARY);
    if (fd < 0) {
        CloseHandle(handle);
        free(p);
        db_host_error();
        return NULL;
    }
    size_t length = wcslen(p);
    wchar_t *pattern = malloc((length + 3) * sizeof(wchar_t));
    DbDir *dir = calloc(1, sizeof(*dir));
    if (!pattern || !dir) {
        free(pattern);
        free(dir);
        _close(fd);
        free(p);
        db_set_errno(12);
        return NULL;
    }
    memcpy(pattern, p, length * sizeof(wchar_t));
    size_t at = length;
    if (!at || (pattern[at - 1] != '\\' && pattern[at - 1] != '/'))
        pattern[at++] = '\\';
    pattern[at++] = '*';
    pattern[at] = 0;
    free(p);
    dir->magic = DB_DIR_MAGIC;
    dir->find = INVALID_HANDLE_VALUE;
    dir->fd = fd;
    dir->first = 1;
    dir->pattern = pattern;
    return dir;
}
static G int db_readdir_r(DbDir *dir, DbDirent *entry, DbDirent **result) {
    if (!dir || dir->magic != DB_DIR_MAGIC || !entry || !result) {
        if (result)
            *result = NULL;
        return 22;
    }
    *result = NULL;
    WIN32_FIND_DATAW data;
    if (dir->first) {
        dir->find = FindFirstFileW(dir->pattern, &data);
        dir->first = 0;
        if (dir->find == INVALID_HANDLE_VALUE) {
            DWORD error = GetLastError();
            if (error == ERROR_FILE_NOT_FOUND)
                return 0;
            win_error();
            return db_get_errno();
        }
    } else {
        if (dir->done)
            return 0;
        if (!FindNextFileW(dir->find, &data)) {
            DWORD error = GetLastError();
            if (error == ERROR_NO_MORE_FILES) {
                dir->done = 1;
                return 0;
            }
            win_error();
            return db_get_errno();
        }
    }
    char name[1024];
    int count = WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, data.cFileName, -1, name,
                                    sizeof(name), NULL, NULL);
    if (count <= 0 || (size_t)count > sizeof(entry->d_name)) {
        db_set_errno(63);
        return 63;
    }
    memset(entry, 0, sizeof(*entry));
    size_t name_length = (size_t)count - 1;
    entry->d_namlen = (uint16_t)name_length;
    entry->d_reclen = (uint16_t)((21 + name_length + 1 + 7) & ~(size_t)7);
    if (data.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY)
        entry->d_type = 4; /* DT_DIR */
    else if (data.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT)
        entry->d_type = 10; /* DT_LNK */
    else
        entry->d_type = 8; /* DT_REG */
    memcpy(entry->d_name, name, name_length + 1);
    *result = entry;
    return 0;
}
static G int db_closedir(DbDir *dir) {
    if (!dir || dir->magic != DB_DIR_MAGIC) {
        db_set_errno(9);
        return -1;
    }
    if (dir->find != INVALID_HANDLE_VALUE)
        FindClose(dir->find);
    free(dir->pattern);
    int result = _close(dir->fd);
    dir->magic = 0;
    free(dir);
    if (result)
        db_host_error();
    return result;
}
static G int db_dirfd(DbDir *dir) {
    if (!dir || dir->magic != DB_DIR_MAGIC) {
        db_set_errno(9);
        return -1;
    }
    return dir->fd;
}
typedef struct DbRegion {
    struct DbRegion *next;
    void *base;
    void *view;
    size_t length;
    int anonymous;
} DbRegion;
#define DB_MAP_FAILED ((void *)(intptr_t)-1)
static DbRegion *regions;
static SRWLOCK region_lock = SRWLOCK_INIT;
static DWORD protection(int prot) {
    int value = prot & 7;
    if (!value)
        return PAGE_NOACCESS;
    if (value & 4)
        return value & 2 ? PAGE_EXECUTE_READWRITE : PAGE_EXECUTE_READ;
    return value & 2 ? PAGE_READWRITE : PAGE_READONLY;
}
static int remember_region(void *base, void *view, size_t length, int anonymous) {
    DbRegion *region = malloc(sizeof(*region));
    if (!region) {
        db_set_errno(12);
        return -1;
    }
    region->base = base;
    region->view = view;
    region->length = length;
    region->anonymous = anonymous;
    AcquireSRWLockExclusive(&region_lock);
    region->next = regions;
    regions = region;
    ReleaseSRWLockExclusive(&region_lock);
    return 0;
}
static G void *db_mmap(void *address, size_t length, int prot, int flags, int fd, int64_t offset) {
    if (!length || offset < 0) {
        db_set_errno(22);
        return DB_MAP_FAILED;
    }
    if (flags & 0x10) { /* MAP_FIXED */
        /* Rust's startup guard-page setup re-maps the lowest stack page at an
         * exact address. That address is already inside the reserved host
         * stack, so commit it in place instead of replacing the stack; the
         * follow-up mprotect(PROT_NONE) then models Darwin's guard. */
        if (!address) {
            db_set_errno(22);
            return DB_MAP_FAILED;
        }
        if (!VirtualAlloc(address, length, MEM_COMMIT, protection(prot))) {
            win_error();
            return DB_MAP_FAILED;
        }
        return address;
    }
    if (flags & 0x1000) { /* MAP_ANON */
        void *mapped = VirtualAlloc(NULL, length, MEM_RESERVE | MEM_COMMIT, protection(prot));
        if (!mapped) {
            win_error();
            return DB_MAP_FAILED;
        }
        if (remember_region(mapped, mapped, length, 1)) {
            VirtualFree(mapped, 0, MEM_RELEASE);
            return DB_MAP_FAILED;
        }
        return mapped;
    }
    if (fd < 0) {
        db_set_errno(9);
        return DB_MAP_FAILED;
    }
    intptr_t raw = _get_osfhandle(fd);
    if (raw == -1) {
        db_set_errno(9);
        return DB_MAP_FAILED;
    }
    SYSTEM_INFO info;
    GetSystemInfo(&info);
    uint64_t alignment = info.dwAllocationGranularity;
    uint64_t aligned = (uint64_t)offset & ~(alignment - 1);
    size_t delta = (size_t)((uint64_t)offset - aligned);
    if (length > SIZE_MAX - delta) {
        db_set_errno(12);
        return DB_MAP_FAILED;
    }
    size_t map_length = length + delta;
    int writable = (prot & 2) != 0;
    DWORD section_protection =
        writable ? ((flags & 1) ? PAGE_READWRITE : PAGE_WRITECOPY) : PAGE_READONLY;
    uint64_t end = aligned + map_length;
    HANDLE section = CreateFileMappingW((HANDLE)raw, NULL, section_protection, (DWORD)(end >> 32),
                                        (DWORD)end, NULL);
    if (!section) {
        win_error();
        return DB_MAP_FAILED;
    }
    DWORD access = (prot & 1 ? FILE_MAP_READ : 0) | (prot & 4 ? FILE_MAP_EXECUTE : 0);
    if (writable)
        access |= (flags & 1) ? FILE_MAP_WRITE : FILE_MAP_COPY;
    if (!access)
        access = FILE_MAP_READ;
    void *view = MapViewOfFile(section, access, (DWORD)(aligned >> 32), (DWORD)aligned, map_length);
    CloseHandle(section);
    if (!view) {
        win_error();
        return DB_MAP_FAILED;
    }
    void *base = (char *)view + delta;
    if (remember_region(base, view, length, 0)) {
        UnmapViewOfFile(view);
        return DB_MAP_FAILED;
    }
    return base;
}
static G int db_munmap(void *address, size_t length) {
    if (!address) {
        db_set_errno(22);
        return -1;
    }
    AcquireSRWLockExclusive(&region_lock);
    DbRegion **link = &regions;
    while (*link) {
        DbRegion *region = *link;
        if (region->base == address && region->length == length) {
            *link = region->next;
            ReleaseSRWLockExclusive(&region_lock);
            int result = region->anonymous ? (VirtualFree(region->view, 0, MEM_RELEASE) ? 0 : -1)
                                           : (UnmapViewOfFile(region->view) ? 0 : -1);
            free(region);
            if (result)
                win_error();
            return result;
        }
        link = &region->next;
    }
    ReleaseSRWLockExclusive(&region_lock);
    db_set_errno(22);
    return -1;
}
static G int db_mprotect(void *address, size_t length, int prot) {
    if (!address || !length) {
        db_set_errno(22);
        return -1;
    }
    DWORD previous;
    if (!VirtualProtect(address, length, protection(prot), &previous)) {
        win_error();
        return -1;
    }
    return 0;
}
static G int db_madvise(void *address, size_t length, int advice) {
    (void)address;
    (void)length;
    (void)advice;
    return 0; /* Advisory only; host paging policy is left to Windows. */
}
typedef struct {
    void *pointer;
    void *base;
    size_t size, alignment;
} DbAlignment;
#define DB_ALIGN_SLOTS 4096
static DbAlignment alignments[DB_ALIGN_SLOTS];
static SRWLOCK alignment_lock = SRWLOCK_INIT;
static size_t alignment_hash(void *pointer) {
    uint64_t value = (uint64_t)(uintptr_t)pointer;
    value ^= value >> 33;
    value *= 0xff51afd7ed558ccdULL;
    value ^= value >> 33;
    return (size_t)value & (DB_ALIGN_SLOTS - 1);
}
static G int db_posix_memalign(void **out, size_t alignment, size_t size) {
    if (!out)
        return 22;
    *out = NULL;
    if (alignment < sizeof(void *) || (alignment & (alignment - 1)))
        return 22;
    /* The returned pointer is advanced into the block, so reserve room for
     * both the worst-case shift and the caller's full requested size. */
    if (size > SIZE_MAX - alignment)
        return 12;
    void *base = malloc(size + alignment);
    if (!base)
        return 12;
    uintptr_t aligned = ((uintptr_t)base + sizeof(void *) + alignment - 1) & ~(alignment - 1);
    AcquireSRWLockExclusive(&alignment_lock);
    size_t slot = alignment_hash((void *)aligned);
    size_t start = slot;
    while (alignments[slot].pointer) {
        slot = (slot + 1) & (DB_ALIGN_SLOTS - 1);
        if (slot == start) {
            ReleaseSRWLockExclusive(&alignment_lock);
            free(base);
            return 12;
        }
    }
    alignments[slot] = (DbAlignment){(void *)aligned, base, size, alignment};
    ReleaseSRWLockExclusive(&alignment_lock);
    *out = (void *)aligned;
    return 0;
}
static int alignment_slot(void *pointer) {
    size_t slot = alignment_hash(pointer);
    size_t start = slot;
    while (alignments[slot].pointer) {
        if (alignments[slot].pointer == pointer)
            return (int)slot;
        slot = (slot + 1) & (DB_ALIGN_SLOTS - 1);
        if (slot == start)
            break;
    }
    return -1;
}
static G void db_free(void *pointer) {
    if (!pointer)
        return;
    AcquireSRWLockExclusive(&alignment_lock);
    int slot = alignment_slot(pointer);
    if (slot < 0) {
        ReleaseSRWLockExclusive(&alignment_lock);
        free(pointer);
        return;
    }
    void *base = alignments[slot].base;
    alignments[slot].pointer = NULL;
    /* Reinsert the following cluster so probing never stops on this hole. */
    size_t next = ((size_t)slot + 1) & (DB_ALIGN_SLOTS - 1);
    while (alignments[next].pointer) {
        DbAlignment entry = alignments[next];
        alignments[next].pointer = NULL;
        size_t target = alignment_hash(entry.pointer);
        while (alignments[target].pointer)
            target = (target + 1) & (DB_ALIGN_SLOTS - 1);
        alignments[target] = entry;
        next = (next + 1) & (DB_ALIGN_SLOTS - 1);
    }
    ReleaseSRWLockExclusive(&alignment_lock);
    free(base);
}
static G void *db_aligned_realloc(void *pointer, size_t size) {
    if (!pointer)
        return malloc(size);
    AcquireSRWLockExclusive(&alignment_lock);
    int slot = alignment_slot(pointer);
    if (slot < 0) {
        ReleaseSRWLockExclusive(&alignment_lock);
        return realloc(pointer, size);
    }
    size_t alignment = alignments[slot].alignment, old_size = alignments[slot].size;
    void *base = alignments[slot].base;
    ReleaseSRWLockExclusive(&alignment_lock);
    if (size > SIZE_MAX - alignment) {
        db_set_errno(12);
        return NULL;
    }
    void *next = malloc(size + alignment);
    if (!next) {
        db_set_errno(12);
        return NULL;
    }
    uintptr_t aligned = ((uintptr_t)next + sizeof(void *) + alignment - 1) & ~(alignment - 1);
    memcpy((void *)aligned, pointer, old_size < size ? old_size : size);
    AcquireSRWLockExclusive(&alignment_lock);
    slot = alignment_slot(pointer);
    if (slot >= 0) {
        alignments[slot].pointer = (void *)aligned;
        alignments[slot].base = next;
        alignments[slot].size = size;
    }
    ReleaseSRWLockExclusive(&alignment_lock);
    free(base);
    return (void *)aligned;
}
static G void *db_realloc_guest(void *pointer, size_t size) {
    if (!pointer)
        return malloc(size);
    AcquireSRWLockShared(&alignment_lock);
    int aligned = alignment_slot(pointer) >= 0;
    ReleaseSRWLockShared(&alignment_lock);
    if (aligned)
        return db_aligned_realloc(pointer, size);
    void *result = realloc(pointer, size);
    if (!result && size)
        db_set_errno(12);
    return result;
}
typedef struct DbSemaphore {
    struct DbSemaphore *next;
    HANDLE handle;
} DbSemaphore;
static DbSemaphore *semaphores;
static SRWLOCK semaphore_lock = SRWLOCK_INIT;
static uint64_t monotonic_ns(void) {
    LARGE_INTEGER counter, frequency;
    if (!QueryPerformanceCounter(&counter) || !QueryPerformanceFrequency(&frequency))
        return 0;
    return (uint64_t)(counter.QuadPart / frequency.QuadPart) * 1000000000 +
           (uint64_t)(counter.QuadPart % frequency.QuadPart) * 1000000000 /
               (uint64_t)frequency.QuadPart;
}
static G void *db_dispatch_semaphore_create(int64_t value) {
    if (value < 0 || value > LONG_MAX) {
        db_set_errno(22);
        return NULL;
    }
    HANDLE handle = CreateSemaphoreW(NULL, (LONG)value, LONG_MAX, NULL);
    DbSemaphore *semaphore = malloc(sizeof(*semaphore));
    if (!handle || !semaphore) {
        if (handle)
            CloseHandle(handle);
        free(semaphore);
        db_set_errno(12);
        return NULL;
    }
    semaphore->handle = handle;
    AcquireSRWLockExclusive(&semaphore_lock);
    semaphore->next = semaphores;
    semaphores = semaphore;
    ReleaseSRWLockExclusive(&semaphore_lock);
    return semaphore;
}
static G int64_t db_dispatch_semaphore_signal(void *object) {
    if (!object) {
        db_set_errno(22);
        return 0;
    }
    AcquireSRWLockShared(&semaphore_lock);
    DbSemaphore *semaphore = semaphores;
    while (semaphore && semaphore != object)
        semaphore = semaphore->next;
    ReleaseSRWLockShared(&semaphore_lock);
    if (!semaphore) {
        db_set_errno(22);
        return 0;
    }
    return ReleaseSemaphore(semaphore->handle, 1, NULL) ? 1 : 0;
}
static G int64_t db_dispatch_semaphore_wait(void *object, uint64_t deadline) {
    if (!object) {
        db_set_errno(22);
        return -1;
    }
    AcquireSRWLockShared(&semaphore_lock);
    DbSemaphore *semaphore = semaphores;
    while (semaphore && semaphore != object)
        semaphore = semaphore->next;
    ReleaseSRWLockShared(&semaphore_lock);
    if (!semaphore) {
        db_set_errno(22);
        return -1;
    }
    DWORD milliseconds;
    if (deadline == UINT64_MAX)
        milliseconds = INFINITE;
    else {
        uint64_t now = monotonic_ns();
        uint64_t wait = deadline > now ? deadline - now : 0;
        uint64_t rounded = (wait + 999999) / 1000000;
        milliseconds = rounded >= INFINITE ? INFINITE - 1 : (DWORD)rounded;
    }
    DWORD result = WaitForSingleObject(semaphore->handle, milliseconds);
    if (result == WAIT_OBJECT_0)
        return 0;
    if (result == WAIT_TIMEOUT)
        return 60; /* Darwin ETIMEDOUT */
    db_set_errno(22);
    return -1;
}
static G uint64_t db_dispatch_time(uint64_t when, int64_t delta) {
    if (when == UINT64_MAX)
        return UINT64_MAX;
    if (!when)
        when = monotonic_ns();
    if (delta > 0) {
        uint64_t add = (uint64_t)delta;
        return when > UINT64_MAX - add ? UINT64_MAX : when + add;
    }
    if (delta < 0) {
        uint64_t subtract = (uint64_t)(-(delta + 1)) + 1;
        return when > subtract ? when - subtract : 0;
    }
    return when;
}
static G void db_dispatch_release(void *object) {
    if (!object)
        return;
    AcquireSRWLockExclusive(&semaphore_lock);
    DbSemaphore **link = &semaphores;
    while (*link && *link != object)
        link = &(*link)->next;
    DbSemaphore *semaphore = *link;
    if (semaphore)
        *link = semaphore->next;
    ReleaseSRWLockExclusive(&semaphore_lock);
    if (semaphore) {
        CloseHandle(semaphore->handle);
        free(semaphore);
    }
}
typedef int (*DbUnwindTrace)(void *context, void *argument);
static G int db_unwind_backtrace(DbUnwindTrace trace, void *argument) {
    /* Guest frames require a Darwin DWARF walker. Reporting the end of the
     * stack keeps diagnostics honest instead of inventing host frames. */
    (void)trace;
    (void)argument;
    return 5; /* _URC_END_OF_STACK */
}
static G uintptr_t db_unwind_get(const void *context) {
    (void)context;
    return 0;
}
static G uintptr_t db_unwind_get_ip_info(const void *context, int *before) {
    (void)context;
    if (before)
        *before = 0;
    return 0;
}
static G void db_unwind_set(const void *context, int index, uintptr_t value) {
    (void)context;
    (void)index;
    (void)value;
}
static G void db_unwind_set_ip(const void *context, uintptr_t value) {
    (void)context;
    (void)value;
}
static G void db_unwind_resume(void *exception) {
    (void)exception;
    fputs("adapter-run [unwind]: guest requested unwinding without a Darwin unwinder\n", stderr);
    db_guest_exit(134);
}
uintptr_t db_system_symbol(const char *name) {
    LOOKUP("_NSGetExecutablePath", db_NSGetExecutablePath);
    LOOKUP("__NSGetExecutablePath", db_NSGetExecutablePath);
    LOOKUP("_NSGetArgc", db_NSGetArgc);
    LOOKUP("__NSGetArgc", db_NSGetArgc);
    LOOKUP("_NSGetArgv", db_NSGetArgv);
    LOOKUP("__NSGetArgv", db_NSGetArgv);
    LOOKUP("_NSGetEnviron", db_NSGetEnviron);
    LOOKUP("__NSGetEnviron", db_NSGetEnviron);
    LOOKUP("_CCRandomGenerateBytes", db_CCRandomGenerateBytes);
    LOOKUP("_clock_gettime", db_clock_gettime);
    LOOKUP("_nanosleep", db_nanosleep);
    LOOKUP("_sched_yield", db_sched_yield);
    LOOKUP("_getcwd", db_getcwd);
    LOOKUP("_chdir", db_chdir);
    LOOKUP("_chroot", db_chroot);
    LOOKUP("_sysconf", db_sysconf);
    LOOKUP("_uname", db_uname);
    LOOKUP("_strerror_r", db_strerror_r);
    LOOKUP("_getpwuid_r", db_getpwuid_r);
    LOOKUP("___memmove_chk", db_memmove_chk);
    LOOKUP("__exit", db_immediate_exit);
    LOOKUP("__Unwind_Backtrace", db_unwind_backtrace);
    LOOKUP("__Unwind_GetCFA", db_unwind_get);
    LOOKUP("__Unwind_GetDataRelBase", db_unwind_get);
    LOOKUP("__Unwind_GetIP", db_unwind_get);
    LOOKUP("__Unwind_GetIPInfo", db_unwind_get_ip_info);
    LOOKUP("__Unwind_GetLanguageSpecificData", db_unwind_get);
    LOOKUP("__Unwind_GetRegionStart", db_unwind_get);
    LOOKUP("__Unwind_GetTextRelBase", db_unwind_get);
    LOOKUP("__Unwind_Resume", db_unwind_resume);
    LOOKUP("__Unwind_SetGR", db_unwind_set);
    LOOKUP("__Unwind_SetIP", db_unwind_set_ip);
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
    LOOKUP("_getpid", db_getpid);
    LOOKUP("_getuid", db_getuid);
    LOOKUP("_geteuid", db_geteuid);
    LOOKUP("_setuid", db_setuid);
    LOOKUP("_setgid", db_setgid);
    LOOKUP("_seteuid", db_seteuid);
    LOOKUP("_setegid", db_setegid);
    LOOKUP("_chown", db_chown);
    LOOKUP("_lchown", db_lchown);
    LOOKUP("_access", db_access);
    LOOKUP("_chmod", db_chmod);
    LOOKUP("_mkdir", db_mkdir);
    LOOKUP("_unlink", db_unlink);
    LOOKUP("_lstat$INODE64", db_stat);
    LOOKUP("_rmdir", db_rmdir);
    LOOKUP("_rename", db_rename);
    LOOKUP("_write", db_write);
    LOOKUP("_read", db_read);
    LOOKUP("_writev", db_writev);
    LOOKUP("_lseek", db_lseek);
    LOOKUP("_close$NOCANCEL", db_close);
    LOOKUP("_dup", db_dup);
    LOOKUP("_dup2", db_dup2);
    LOOKUP("_pipe", db_pipe);
    LOOKUP("_fcntl", db_fcntl);
    LOOKUP("_ioctl", db_ioctl);
    LOOKUP("_fstatat$INODE64", db_fstatat);
    LOOKUP("_opendir$INODE64", db_opendir);
    LOOKUP("_readdir_r$INODE64", db_readdir_r);
    LOOKUP("_closedir", db_closedir);
    LOOKUP("_dirfd", db_dirfd);
    LOOKUP("_mmap", db_mmap);
    LOOKUP("_munmap", db_munmap);
    LOOKUP("_mprotect", db_mprotect);
    LOOKUP("_madvise", db_madvise);
    LOOKUP("_posix_madvise", db_madvise);
    LOOKUP("_posix_memalign", db_posix_memalign);
    LOOKUP("_realloc", db_realloc_guest);
    LOOKUP("_free", db_free);
    LOOKUP("_dispatch_release", db_dispatch_release);
    LOOKUP("_dispatch_semaphore_create", db_dispatch_semaphore_create);
    LOOKUP("_dispatch_semaphore_signal", db_dispatch_semaphore_signal);
    LOOKUP("_dispatch_semaphore_wait", db_dispatch_semaphore_wait);
    LOOKUP("_dispatch_time", db_dispatch_time);
    LOOKUP("_sigaction", db_sigaction);
    LOOKUP("_sigemptyset", db_sigemptyset);
    LOOKUP("_sigaddset", db_sigaddset);
    LOOKUP("_sigaltstack", db_sigaltstack);
    LOOKUP("_signal", db_signal);
    LOOKUP("_pause", db_pause);
    LOOKUP("_fork", db_fork);
    LOOKUP("_execvp", db_execvp);
    LOOKUP("_waitpid", db_waitpid);
    LOOKUP("_setpgid", db_setpgid);
    LOOKUP("_setsid", db_setsid);
    LOOKUP("_setgroups", db_setgroups);
    LOOKUP("_posix_spawn_file_actions_init", db_spawn_file_actions_init);
    LOOKUP("_posix_spawn_file_actions_destroy", db_spawn_file_actions_destroy);
    LOOKUP("_posix_spawn_file_actions_adddup2", db_spawn_file_actions_adddup2);
    LOOKUP("_posix_spawnattr_init", db_spawn_attr_init);
    LOOKUP("_posix_spawnattr_destroy", db_spawn_attr_destroy);
    LOOKUP("_posix_spawnattr_setflags", db_spawn_attr_setflags);
    LOOKUP("_posix_spawnattr_setpgroup", db_spawn_attr_setpgroup);
    LOOKUP("_posix_spawnattr_setsigdefault", db_spawn_attr_setsigdefault);
    LOOKUP("_posix_spawnp", db_posix_spawnp);
    LOOKUP("_memset_pattern16", db_memset_pattern16);
    LOOKUP("_sleep", db_sleep);
    LOOKUP("_usleep", db_usleep);
    LOOKUP("_srandom", db_srandom);
    LOOKUP("_srandomdev", db_srandomdev);
    LOOKUP("_random", db_random);
    LOOKUP("_bsearch", db_bsearch);
    LOOKUP("_ftell", db_ftell);
    LOOKUP("_arc4random_uniform", db_arc4random_uniform);
    LOOKUP("_mkdtemp", db_mkdtemp);
    LOOKUP("_gethostname", db_gethostname);
    LOOKUP("_umask", db_umask);
    LOOKUP("_utimes", db_utimes);
    LOOKUP("_localtime", db_localtime1);
    return 0;
}
