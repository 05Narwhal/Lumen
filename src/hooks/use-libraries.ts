import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { invalidateEverything } from "@/lib/events";
import { invoke } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";
import type { LibraryEntry } from "@/types/generated/LibraryEntry";
import type { LibraryInfo } from "@/types/generated/LibraryInfo";
import type { LibraryListing } from "@/types/generated/LibraryListing";
import type { LibraryProblem } from "@/types/generated/LibraryProblem";

export function useLibraries() {
  const qc = useQueryClient();
  const listing = useQuery({
    queryKey: queryKeys.libraries,
    queryFn: () => invoke<LibraryListing>("list_libraries"),
    staleTime: Infinity,
  });
  const current = useQuery({
    queryKey: queryKeys.currentLibrary,
    queryFn: () => invoke<LibraryInfo | null>("get_current_library"),
    staleTime: Infinity,
  });
  const missing = useQuery({
    queryKey: queryKeys.missingLibrary,
    queryFn: () => invoke<LibraryEntry | null>("get_missing_library"),
    staleTime: Infinity,
  });
  const problem = useQuery({
    queryKey: queryKeys.libraryProblem,
    queryFn: () => invoke<LibraryProblem | null>("get_library_problem"),
    staleTime: Infinity,
  });
  // Seed the current-library cache synchronously so a redirect can never see stale null,
  // then drop every library-derived query: a different library means different data.
  const adopted = (info: LibraryInfo) => {
    qc.setQueryData(queryKeys.currentLibrary, info);
    qc.setQueryData(queryKeys.missingLibrary, null);
    qc.setQueryData(queryKeys.libraryProblem, null);
    invalidateEverything(qc);
  };
  const create = useMutation({
    mutationFn: (path: string) => invoke<LibraryInfo>("create_library", { path }),
    onSuccess: adopted,
  });
  const open = useMutation({
    mutationFn: (path: string) => invoke<LibraryInfo>("open_library", { path }),
    onSuccess: adopted,
  });
  const locate = useMutation({
    mutationFn: (args: { id: string; path: string }) => invoke<LibraryInfo>("locate_library", args),
    onSuccess: adopted,
  });
  const forget = useMutation({
    mutationFn: (id: string) => invoke<LibraryListing>("forget_library", { id }),
    onSuccess: (l) => qc.setQueryData(queryKeys.libraries, l),
  });
  return {
    listing: listing.data,
    current: current.data ?? null,
    missing: missing.data ?? null,
    problem: problem.data ?? null,
    isLoading: listing.isLoading || current.isLoading || missing.isLoading || problem.isLoading,
    /** All four queries have settled at least once. Only then is `current === null` meaningful. */
    isLoaded: listing.isSuccess && current.isSuccess && missing.isSuccess && problem.isSuccess,
    createLibrary: create.mutateAsync,
    openLibrary: open.mutateAsync,
    locateLibrary: locate.mutateAsync,
    forgetLibrary: forget.mutateAsync,
    existsAt: (path: string) => invoke<boolean>("library_exists_at", { path }),
  };
}
