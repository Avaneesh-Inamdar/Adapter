extern int tlv_register_check(void);
extern int tlv_avx_check(void);
extern int puts(const char *);
__thread int register_value=73;
int main(void) {
    if (tlv_register_check()) return 121;
    int avx=tlv_avx_check();
    if (avx==1) return 122;
    puts(avx==2 ? "tls:register preservation passed (AVX unavailable)" : "tls:register preservation passed");return 0;
}
