extern void *memcpy(void *,const void *,unsigned long);
extern unsigned long strlen(const char *);
static volatile double factor=1.25;
__attribute__((noinline)) static double calculation(double a,double b,double c,double d,double e,double f,double g,double h,double i) {
    return (a+b+c+d+e+f+g+h+i)*factor;
}
__attribute__((noinline)) static long integers(long a,long b,long c,long d,long e,long f,long g,long h) {
    return a+2*b+3*c+4*d+5*e+6*f+7*g+8*h;
}
int main(void) {
    char s[8];
    memcpy(s,"abcdefg",8);
    if (strlen(s)!=7 || calculation(1,2,3,4,5,6,7,8,9)!=56.25 || integers(1,2,3,4,5,6,7,8)!=204) return 94;
    return 0;
}
