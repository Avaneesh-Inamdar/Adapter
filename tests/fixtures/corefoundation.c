typedef const void *CFStringRef;
extern CFStringRef CFStringCreateWithCString(const void *, const char *, unsigned int);
extern long CFStringGetLength(CFStringRef);
extern unsigned char CFStringGetCString(CFStringRef,char *,long,unsigned int);
extern const void *CFRetain(const void *);
extern void CFRelease(const void *);
extern int puts(const char *);
extern int memcmp(const void *,const void *,unsigned long);
int main(void) {
    const unsigned int utf8=0x08000100;
    CFStringRef s=CFStringCreateWithCString((void*)0,"A\xc3\xa9\xf0\x9f\x98\x80",utf8);
    if (!s || CFStringGetLength(s)!=4) return 81; /* UTF-16 units, not UTF-8 bytes. */
    if (CFRetain(s)!=s) return 82;
    CFRelease(s);
    char small[7], out[8];
    if (CFStringGetCString(s,small,7,utf8)) return 83;
    if (!CFStringGetCString(s,out,8,utf8) || memcmp(out,"A\xc3\xa9\xf0\x9f\x98\x80",8)) return 84;
    if (CFStringCreateWithCString((void*)0,"\xff",utf8)) return 85;
    CFRelease(s);
    puts("DarwinBridge: CoreFoundation UTF-8 / UTF-16 checks passed");
    return 0;
}
