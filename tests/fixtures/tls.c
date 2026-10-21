extern int memcmp(const void *,const void *,unsigned long);
extern int puts(const char *);
__thread int initialized=37;
__thread int zero;
__thread char aligned[64] __attribute__((aligned(64)));
__thread const char *text="thread-local";
int main(void) {
    if (initialized!=37 || zero!=0 || ((unsigned long)aligned&63)!=0) return 101;
    if (memcmp(text,"thread-local",13)) return 102;
    initialized+=5;zero=9;aligned[63]=11;
    if (initialized!=42 || zero!=9 || aligned[63]!=11) return 103;
    puts("tls:passed");return 0;
}
