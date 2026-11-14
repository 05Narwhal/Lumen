import { Button, Modal, TextInput } from "@mantine/core";
import { IconArrowUp, IconFolder, IconPlus } from "@tabler/icons-react";
import { useState } from "react";
import { useAlbumCollections } from "@/hooks/use-album-collections";
import { moveTargets } from "@/lib/album-collections";
import { closeCollectionPicker, collectionPickerStore } from "./store";
import "./styles.scss";

/**
 * *Move to collection…* (Round 5A): every collection as an indented tree, *No collection*
 * (the top) when the thing is in one, and a new collection made on the spot. A collection
 * is never offered itself or one of its own.
 */
export function CollectionPicker({
  title,
  moving,
  current,
  onPick,
  onClose,
}: {
  title: string;
  moving: { album: number } | { collection: number };
  /** Where it is now: not offered again. */
  current: number | null;
  /** The chosen collection, or `null` for the top. */
  onPick: (collection: number | null) => void;
  onClose: () => void;
}) {
  const { collections, create } = useAlbumCollections();
  const [creating, setCreating] = useState(false);
  const [name, setName] = useState("");
  const targets = moveTargets(collections, moving).filter((t) => t.collection.id !== current);

  const createAndPick = async () => {
    if (!name.trim()) return;
    const id = await create({ name: name.trim(), parentId: null });
    onPick(id);
  };

  return (
    <Modal opened onClose={onClose} title={title} centered size="sm" classNames={{ content: "collection-picker", header: "collection-picker__header", body: "collection-picker__body" }}>
      <ul className="collection-picker__list" role="listbox" aria-label="Collections">
        {current != null && (
          <li>
            <button type="button" role="option" aria-selected={false} className="collection-picker__row" onClick={() => onPick(null)}>
              <IconArrowUp size={16} aria-hidden />
              <span className="collection-picker__name">No collection</span>
            </button>
          </li>
        )}
        {targets.map(({ collection, depth }) => (
          <li key={collection.id}>
            <button
              type="button"
              role="option"
              aria-selected={false}
              className="collection-picker__row"
              style={{ "--depth": depth } as React.CSSProperties}
              onClick={() => onPick(collection.id)}
            >
              <IconFolder size={16} aria-hidden />
              <span className="collection-picker__name">{collection.name}</span>
            </button>
          </li>
        ))}
        {targets.length === 0 && current == null && <li className="collection-picker__empty">No collections yet.</li>}
      </ul>
      {creating ? (
        <form
          className="collection-picker__new"
          onSubmit={(e) => {
            e.preventDefault();
            void createAndPick();
          }}
        >
          <TextInput autoFocus data-autofocus placeholder="Collection name" aria-label="Collection name" value={name} onChange={(e) => setName(e.currentTarget.value)} />
          <Button type="submit" disabled={!name.trim()}>
            Create
          </Button>
        </form>
      ) : (
        <Button variant="subtle" leftSection={<IconPlus size={16} />} onClick={() => setCreating(true)} className="collection-picker__add">
          New collection…
        </Button>
      )}
    </Modal>
  );
}

/** The picker for moves started where no menu is (the palette). Mount once. */
export function CollectionPickerHost() {
  const { album } = collectionPickerStore.use();
  const { moveAlbum } = useAlbumCollections();
  if (!album) return null;
  return (
    <CollectionPicker
      title={`Move “${album.name}” to…`}
      moving={{ album: album.id }}
      current={album.collectionId}
      onClose={closeCollectionPicker}
      onPick={(collectionId) => {
        closeCollectionPicker();
        void moveAlbum({ albumId: album.id, collectionId, albumName: album.name });
      }}
    />
  );
}
