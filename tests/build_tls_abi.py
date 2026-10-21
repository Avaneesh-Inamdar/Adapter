"""Produce assembly probes that retain live registers across a TLV thunk call."""
from build_fixtures import ROOT,OUT,tool,run
from build_dylib_fixtures import compile_source

def build():
    out=OUT/'tls-abi';out.mkdir(exist_ok=True)
    save='pushq %rbp\npushq %rbx\npushq %r12\npushq %r13\npushq %r14\npushq %r15\nsubq $8,%rsp\n'
    restore='addq $8,%rsp\npopq %r15\npopq %r14\npopq %r13\npopq %r12\npopq %rbx\npopq %rbp\nretq\n'
    registers=['rsi','rdx','rcx','r8','r9','r10','r11','rbx','rbp','r12','r13','r14','r15']
    asm='.text\n.globl _tlv_register_check\n_tlv_register_check:\n'+save
    asm+='movq _register_value@TLVP(%rip),%rdi\nmovq %rdi,(%rsp)\n'
    for n,reg in enumerate(registers,1):asm+=f'movq ${n+100},%{reg}\n'
    for n in range(16):asm+=f'movdqu _pattern(%rip),%xmm{n}\n'
    asm+='callq *(%rdi)\ncmpl $73,(%rax)\njne _fail\ncmpq (%rsp),%rdi\njne _fail\n'
    for n,reg in enumerate(registers,1):asm+=f'cmpq ${n+100},%{reg}\njne _fail\n'
    for n in range(16):asm+=f'pcmpeqb _pattern(%rip),%xmm{n}\npmovmskb %xmm{n},%eax\ncmpl $65535,%eax\njne _fail\n'
    asm+='xorl %eax,%eax\njmp _return\n_fail:\nmovl $1,%eax\n_return:\n'+restore
    asm+='.globl _tlv_avx_check\n_tlv_avx_check:\n'+save
    asm+='movl $1,%eax\ncpuid\nmovl %ecx,%eax\nandl $0x18000000,%eax\ncmpl $0x18000000,%eax\njne _avx_skip\nxorl %ecx,%ecx\nxgetbv\nandl $6,%eax\ncmpl $6,%eax\njne _avx_skip\n'
    asm+='movq _register_value@TLVP(%rip),%rdi\n'
    for n in range(16):asm+=f'vmovdqu _wide_pattern(%rip),%ymm{n}\n'
    asm+='callq *(%rdi)\n'
    for n in range(16):asm+=f'vpcmpeqb _wide_pattern(%rip),%ymm{n},%ymm{n}\nvpmovmskb %ymm{n},%eax\ncmpl $-1,%eax\njne _avx_fail\n'
    asm+='vzeroupper\nxorl %eax,%eax\njmp _avx_return\n_avx_skip:\nmovl $2,%eax\njmp _avx_return\n_avx_fail:\nvzeroupper\nmovl $1,%eax\n_avx_return:\n'+restore
    asm+='.section __TEXT,__const\n.p2align 5\n_pattern:\n.quad 0x0123456789abcdef,0xfedcba9876543210\n_wide_pattern:\n.quad 0x1122334455667788,0x8877665544332211,0x1234567812345678,0x7654321076543210\n'
    source=out/'probe.s';source.write_text(asm)
    run(tool('clang'),'-target','x86_64-apple-macos10.13','-c',source,'-o',out/'probe.o')
    compile_source(ROOT/'tests/fixtures/tls_abi.c',out/'main.o')
    for variant,version,flag in [('classic','10.13','-no_fixup_chains'),('chained','11.0','-fixup_chains')]:
        run(tool('ld64.lld'),'-arch','x86_64','-platform_version','macos',version,version,flag,'-no_uuid','-o',out/variant,out/'main.o',out/'probe.o','-L',OUT,'-lSystem')
    print('Built TLV GPR/XMM/AVX register-preservation probes')
if __name__=='__main__':build()
