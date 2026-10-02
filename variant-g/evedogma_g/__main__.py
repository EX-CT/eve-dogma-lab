import os

# The engine never uses BLAS; a single-threaded OpenBLAS skips spawning one thread per core at import
# (that start-up costs 5-80 ms of every cold start, the more the busier the machine is).
for _v in ("OPENBLAS_NUM_THREADS", "OMP_NUM_THREADS", "MKL_NUM_THREADS"):
    os.environ.setdefault(_v, "1")

from .cli import main  # noqa: E402

main()
