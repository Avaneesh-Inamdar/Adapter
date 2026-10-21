#ifndef DB_BRIDGE_H
#define DB_BRIDGE_H
#include <stdint.h>
#include <stddef.h>
#include <stdio.h>
#define G __attribute__((sysv_abi))
#define LOOKUP(n, f)                                                                               \
    if (!strcmp(name, n))                                                                          \
    return (uintptr_t)(f)

/* Apple LP64 FILE prefix, followed by private host state. All counts stay zero
 * so inline getc/putc take the slow path. The host FILE is a separate object. */
typedef struct {
    unsigned char *base;
    int size;
} DbBuf;
typedef struct DbFile {
    unsigned char *p;
    int r, w;
    short flags, file;
    DbBuf bf;
    int lbfsize;
    void *cookie, *close, *read, *seek, *write;
    DbBuf ub;
    void *extra;
    int ur;
    unsigned char ubuf[3], nbuf[1];
    DbBuf lb;
    int blksize;
    int64_t offset;
    FILE *host;
    int owned;
} DbFile;
_Static_assert(offsetof(DbFile, host) == 152, "Darwin FILE size");
_Static_assert(offsetof(DbFile, flags) == 16, "Darwin FILE flags");
_Static_assert(offsetof(DbFile, file) == 18, "Darwin FILE descriptor");

/* System V AMD64 ABI register-save-area va_list, not a Windows va_list. */
typedef struct {
    uint32_t gp, fp;
    unsigned char *overflow, *registers;
} DbVa;
_Static_assert(sizeof(DbVa) == 24, "SysV va_list");
extern DbFile *db_stdin, *db_stdout, *db_stderr;
void db_set_errno(int value);
int db_get_errno(void);
void db_host_error(void);
G void db_stack_fail(void);
void db_file_status(DbFile *f);
wchar_t *db_path(const char *s);
G void db_guest_exit(int code);
uintptr_t db_format_symbol(const char *name);
uintptr_t db_system_symbol(const char *name);
void db_system_begin(void);
void db_set_executable_path(const char *path);
void db_libc_flush(void);
G int db_fputc(int c, DbFile *f);
G size_t db_fwrite(const void *p, size_t s, size_t n, DbFile *f);
G int db_puts(const char *s);
G char *db_getenv(const char *name);
G int db_setenv(const char *name, const char *value, int overwrite);
G int db_unsetenv(const char *name);
G int db_guest_printf(const char *fmt, ...);
G int db_guest_fprintf(DbFile *file, const char *fmt, ...);
G int db_guest_snprintf(char *out, size_t size, const char *fmt, ...);
G int db_guest_sscanf(const char *str, const char *fmt, ...);
G void db_chkstk(void);
#endif
