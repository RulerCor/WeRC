fn main() {
    // Match the desktop: no separately installed VC runtime DLL is required.
    // The Universal CRT remains an OS dependency on supported Windows versions.
    static_vcruntime::metabuild();
}
