/*
 * Generates small.h5, the fixture for tests/it/hdf5_reader.rs.
 *
 * The file uses libhdf5's default (earliest) file format, like the reference app: superblock v0,
 * v1 object headers, symbol-table groups and v1 B-tree chunk indexes. Dataset bytes follow
 * `pattern(seed, i)` below; the Rust test recomputes them, so keep both in sync.
 *
 * Regenerate (from crates/spp-reader/tests/fixtures):
 *   gcc -O2 -o /tmp/make_fixture make_fixture.c -lhdf5 && /tmp/make_fixture small.h5
 */
#include <hdf5.h>
#include <stdio.h>
#include <stdlib.h>

static unsigned char pattern(unsigned seed, unsigned long i) {
    return (unsigned char)((i * 31u + seed * 7u + (i >> 8)) % 251u);
}

static unsigned char *make_bytes(unsigned seed, size_t len) {
    unsigned char *buf = malloc(len ? len : 1);
    for (size_t i = 0; i < len; i++) buf[i] = pattern(seed, i);
    return buf;
}

/* A 1-D byte dataset with the given layout. chunk == 0 means contiguous, chunk < 0 compact. */
static hid_t make_dataset(hid_t parent, const char *name, hsize_t len, long chunk, int unlimited) {
    hsize_t max = unlimited ? H5S_UNLIMITED : len;
    hid_t space = H5Screate_simple(1, &len, &max);
    hid_t dcpl = H5Pcreate(H5P_DATASET_CREATE);
    if (chunk > 0) {
        hsize_t c = (hsize_t)chunk;
        H5Pset_chunk(dcpl, 1, &c);
    } else if (chunk < 0) {
        H5Pset_layout(dcpl, H5D_COMPACT);
    }
    hid_t dset = H5Dcreate2(parent, name, H5T_STD_U8BE, space, H5P_DEFAULT, dcpl, H5P_DEFAULT);
    H5Pclose(dcpl);
    H5Sclose(space);
    return dset;
}

static void write_dataset(hid_t parent, const char *name, unsigned seed, hsize_t len, long chunk, int unlimited) {
    hid_t dset = make_dataset(parent, name, len, chunk, unlimited);
    if (len > 0) {
        unsigned char *buf = make_bytes(seed, len);
        H5Dwrite(dset, H5T_NATIVE_UCHAR, H5S_ALL, H5S_ALL, H5P_DEFAULT, buf);
        free(buf);
    }
    H5Dclose(dset);
}

/* Like the reference app: an attribute `m3_x64_128` holding two u64. */
static void add_attribute(hid_t obj) {
    hsize_t two = 2;
    unsigned long long value[2] = {0x0123456789abcdefULL, 42};
    hid_t space = H5Screate_simple(1, &two, NULL);
    hid_t attr = H5Acreate2(obj, "m3_x64_128", H5T_STD_U64BE, space, H5P_DEFAULT, H5P_DEFAULT);
    H5Awrite(attr, H5T_NATIVE_ULLONG, value);
    H5Aclose(attr);
    H5Sclose(space);
}

int main(int argc, char **argv) {
    const char *out = argc > 1 ? argv[1] : "small.h5";
    /* HDF5 2.x defaults to newer structures; force the earliest format, as the reference app writes. */
    hid_t fapl = H5Pcreate(H5P_FILE_ACCESS);
    H5Pset_libver_bounds(fapl, H5F_LIBVER_EARLIEST, H5F_LIBVER_V18);
    hid_t file = H5Fcreate(out, H5F_ACC_TRUNC, H5P_DEFAULT, fapl);
    H5Pclose(fapl);

    hid_t alpha = H5Gcreate2(file, "alpha", H5P_DEFAULT, H5P_DEFAULT, H5P_DEFAULT);
    hid_t beta = H5Gcreate2(alpha, "beta", H5P_DEFAULT, H5P_DEFAULT, H5P_DEFAULT);

    /* 300 chunks of 256 bytes, the last one partial: a multi-level chunk B-tree. */
    hid_t chunked = make_dataset(beta, "chunked.bin", 76700, 256, 1);
    unsigned char *buf = make_bytes(1, 76700);
    H5Dwrite(chunked, H5T_NATIVE_UCHAR, H5S_ALL, H5S_ALL, H5P_DEFAULT, buf);
    free(buf);
    add_attribute(chunked);
    H5Dclose(chunked);

    write_dataset(alpha, "contiguous.bin", 2, 1000, 0, 0);
    write_dataset(alpha, "compact.bin", 3, 50, -1, 0);
    write_dataset(file, "empty.bin", 4, 0, 65536, 1);

    /* Only bytes 1024..1100 written: chunks 0..3 and 5..7 are never allocated and read as zeros. */
    hid_t sparse = make_dataset(alpha, "sparse.bin", 2048, 256, 1);
    hsize_t start = 1024, count = 76;
    hid_t file_space = H5Dget_space(sparse);
    H5Sselect_hyperslab(file_space, H5S_SELECT_SET, &start, NULL, &count, NULL);
    hid_t mem_space = H5Screate_simple(1, &count, NULL);
    buf = make_bytes(5, count);
    H5Dwrite(sparse, H5T_NATIVE_UCHAR, mem_space, file_space, H5P_DEFAULT, buf);
    free(buf);
    H5Sclose(mem_space);
    H5Sclose(file_space);
    H5Dclose(sparse);

    /* Enough members for a multi-level group B-tree (8 symbols per node, 32 nodes per B-tree node). */
    hid_t many = H5Gcreate2(file, "many", H5P_DEFAULT, H5P_DEFAULT, H5P_DEFAULT);
    for (unsigned i = 0; i < 300; i++) {
        char name[32];
        snprintf(name, sizeof name, "item%03u", i);
        write_dataset(many, name, 100 + i, 3 + i % 5, -1, 0);
    }
    H5Gclose(many);

    H5Gclose(beta);
    H5Gclose(alpha);
    H5Fclose(file);
    return 0;
}
