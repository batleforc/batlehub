<script setup lang="ts">
/**
 * "Who is blocked, and why?" (RFC 0004 Phase 5, *merge*).
 *
 * This was two routes — `/admin/security/users` and `/admin/security/ip-blocks`
 * — and an operator arrives with a *symptom* ("the EU CI runner started
 * failing"), not with a mechanism. To answer it they had to visit both, because
 * a principal can be stopped two ways and neither page mentioned the other.
 *
 * The two mechanisms are peers, not layers: the user-block middleware rejects
 * on `Identity.user_id` with 401 and the IP-block middleware rejects on client
 * address with 403, and both run *before* any rule evaluation. So they are one
 * question with two answers, which is a merge rather than two pages that happen
 * to look alike.
 *
 * They also were the same page written twice: the same header, the same single
 * card, the same five-column table, the same two dialogs — differing in four
 * column labels and one form field.
 *
 * One deliberate inheritance: the SDK path won. The users half hand-rolled
 * `authFetch` against `API_BASE_URL` while `listBlockedUsers`/`blockUser`/
 * `unblockUser` were already generated and typed.
 */
import { useI18n } from "vue-i18n";
import { computed, ref } from "vue";
import {
  listBlockedUsers,
  blockUser,
  unblockUser,
  listBlockedIps,
  blockIp,
  unblockIp,
} from "@/client/sdk.gen";
import type { BlockedIpDto, UserBlockDto } from "@/client/types.gen";
import { useApi, extractMessage } from "@/composables/useApi";
import { useAuth } from "@/composables/useAuth";
import { formatDate as fmtDate } from "@/lib/format";
import SectionTabs from "@/components/admin/SectionTabs.vue";
import { SECURITY_TABS } from "@/config/adminSections";
import { PageHeader } from "@/components/ui/page-header";
import { AsyncState } from "@/components/ui/async-state";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { EmptyState } from "@/components/ui/empty-state";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Dialog } from "@/components/ui/dialog";
import { DestructiveConfirm } from "@/components/ui/destructive-confirm";
import {
  Table,
  TableHeader,
  TableBody,
  TableRow,
  TableHead,
  TableCell,
} from "@/components/ui/table";

const { t } = useI18n();
const { token } = useAuth();

type BlockKind = "account" | "ip";

/** One row of the merged table, whichever mechanism produced it. */
interface BlockRow {
  kind: BlockKind;
  /** The user id or the address — what was blocked. */
  subject: string;
  reason: string | null;
  blockedAt: string | null;
  /** Who added it. `null` for machine-created IP blocks. */
  blockedBy: string | null;
  /** IP blocks expire; account blocks do not. */
  unblocksAt: string | null;
  /** True once `unblocksAt` has passed — the row is history, not a block. */
  expired: boolean;
}

const {
  data: users,
  error: usersError,
  loading: usersLoading,
  reload: reloadUsers,
} = useApi<UserBlockDto[]>(() => listBlockedUsers(), [token]);

const {
  data: ips,
  error: ipsError,
  loading: ipsLoading,
  reload: reloadIps,
} = useApi<BlockedIpDto[]>(() => listBlockedIps(), [token]);

const loading = computed(() => usersLoading.value || ipsLoading.value);
const error = computed(() => usersError.value ?? ipsError.value);

/**
 * `Date.now()` read once per render pass rather than per row, so every row in
 * one paint agrees about what "now" is.
 */
const now = ref(Date.now());
function refresh() {
  now.value = Date.now();
  reloadUsers();
  reloadIps();
}

const rows = computed<BlockRow[]>(() => {
  const accountRows: BlockRow[] = (users.value ?? []).map((u) => ({
    kind: "account",
    subject: u.user_id,
    reason: u.reason ?? null,
    blockedAt: u.blocked_at ?? null,
    blockedBy: u.blocked_by ?? null,
    unblocksAt: null,
    expired: false,
  }));

  const ipRows: BlockRow[] = (ips.value ?? []).map((b) => ({
    kind: "ip",
    subject: b.ip,
    reason: b.reason || null,
    blockedAt: b.blocked_at ? new Date(b.blocked_at * 1000).toISOString() : null,
    // `BlockedIpDto` carries no author: an address block is either an
    // operator's or the middleware's, and the `"auto"` reason is what says
    // which. Rendered from the reason column rather than invented here.
    blockedBy: null,
    unblocksAt: b.unblock_at ? new Date(b.unblock_at * 1000).toISOString() : null,
    expired: b.unblock_at ? b.unblock_at * 1000 <= now.value : false,
  }));

  // Live blocks first, then expired history; newest first within each.
  return [...accountRows, ...ipRows].sort((a, b) => {
    if (a.expired !== b.expired) return a.expired ? 1 : -1;
    return (b.blockedAt ?? "").localeCompare(a.blockedAt ?? "");
  });
});

const filter = ref("");
const visibleRows = computed(() => {
  const q = filter.value.trim().toLowerCase();
  if (!q) return rows.value;
  return rows.value.filter(
    (r) => r.subject.toLowerCase().includes(q) || (r.reason ?? "").toLowerCase().includes(q),
  );
});

// ── Adding a block ───────────────────────────────────────────────────────────

const dialogOpen = ref(false);
const dialogKind = ref<BlockKind>("account");
const form = ref({ subject: "", reason: "", durationSecs: 3600 });
const formLoading = ref(false);
const formError = ref<string | null>(null);

function openBlock(kind: BlockKind) {
  dialogKind.value = kind;
  form.value = { subject: "", reason: "", durationSecs: 3600 };
  formError.value = null;
  dialogOpen.value = true;
}

/**
 * Blocking your own address locks you out of the instance you are administering,
 * and nothing warned about it. A warning rather than a refusal: an operator may
 * legitimately be blocking a shared egress address.
 */
const selfBlockRisk = computed(
  () => dialogKind.value === "ip" && form.value.subject.trim().length > 0,
);

async function submitBlock() {
  const subject = form.value.subject.trim();
  if (!subject) return;
  formLoading.value = true;
  formError.value = null;
  try {
    const res =
      dialogKind.value === "account"
        ? await blockUser({
            path: { user_id: subject },
            body: { reason: form.value.reason.trim() || null },
          })
        : await blockIp({
            body: {
              ip: subject,
              reason: form.value.reason.trim() || null,
              duration_secs: form.value.durationSecs,
            },
          });
    if (res.error) {
      formError.value = extractMessage(res.error);
      return;
    }
    dialogOpen.value = false;
    refresh();
  } finally {
    formLoading.value = false;
  }
}

// ── Lifting one ──────────────────────────────────────────────────────────────

const unblockTarget = ref<BlockRow | null>(null);
const unblockLoading = ref(false);
const unblockError = ref<string | null>(null);

async function confirmUnblock() {
  const target = unblockTarget.value;
  if (!target) return;
  unblockLoading.value = true;
  unblockError.value = null;
  try {
    const res =
      target.kind === "account"
        ? await unblockUser({ path: { user_id: target.subject } })
        : await unblockIp({ path: { ip: target.subject } });
    if (res.error) {
      unblockError.value = extractMessage(res.error);
      return;
    }
    unblockTarget.value = null;
    refresh();
  } finally {
    unblockLoading.value = false;
  }
}
</script>

<template>
  <div class="space-y-4">
    <SectionTabs :tabs="SECURITY_TABS" />
    <PageHeader
      variant="display"
      :title="t('adminBlocks.title')"
      :description="t('adminBlocks.description')"
    >
      <template #actions>
        <Button variant="outline" size="sm" :disabled="loading" @click="refresh">
          {{ loading ? t("common.refreshing") : t("common.refresh") }}
        </Button>
        <Button size="sm" @click="openBlock('account')">{{ t("adminBlocks.blockAccount") }}</Button>
        <Button size="sm" variant="outline" @click="openBlock('ip')">{{
          t("adminBlocks.blockAddress")
        }}</Button>
      </template>
    </PageHeader>

    <Input v-model="filter" :placeholder="t('adminBlocks.filter')" class="max-w-sm" />

    <AsyncState :loading="loading && !rows.length" :error="error">
      <EmptyState
        v-if="visibleRows.length === 0"
        :title="filter ? t('adminBlocks.noMatches') : t('adminBlocks.nobodyBlocked')"
        :description="filter ? undefined : t('adminBlocks.nobodyBlockedBody')"
        :filtered="!!filter"
      />
      <Card v-else>
        <CardContent class="p-0">
          <Table :label="t('adminBlocks.title')">
            <TableHeader>
              <TableRow>
                <TableHead>{{ t("adminBlocks.kind") }}</TableHead>
                <TableHead>{{ t("adminBlocks.subject") }}</TableHead>
                <TableHead>{{ t("common.reason") }}</TableHead>
                <TableHead>{{ t("adminBlocks.since") }}</TableHead>
                <TableHead>{{ t("adminBlocks.until") }}</TableHead>
                <TableHead class="text-right">{{ t("common.actions") }}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              <TableRow
                v-for="row in visibleRows"
                :key="`${row.kind}:${row.subject}`"
                :class="row.expired ? 'text-muted-foreground' : ''"
              >
                <TableCell>
                  <Badge :variant="row.kind === 'account' ? 'default' : 'copper'" class="text-xs">
                    {{ row.kind === "account" ? t("adminBlocks.account") : t("adminBlocks.ip") }}
                  </Badge>
                </TableCell>
                <TableCell class="font-mono text-sm">{{ row.subject }}</TableCell>
                <!--
                  Not truncated. The reason is the one column carrying the *why*,
                  and both source pages capped it at 240px on a table with ~500px
                  of unused width — so "Sustained 429s against the n…" ended in
                  empty space.
                -->
                <TableCell class="text-sm">
                  <span v-if="row.reason === 'auto'" class="text-copper">{{
                    t("adminBlocks.automatic")
                  }}</span>
                  <span v-else-if="row.reason">{{ row.reason }}</span>
                  <span v-else class="text-muted-foreground">—</span>
                </TableCell>
                <TableCell class="text-xs">
                  {{ fmtDate(row.blockedAt) }}
                  <span v-if="row.blockedBy" class="block text-muted-foreground"
                    >{{ t("adminBlocks.by", { who: row.blockedBy }) }}</span
                  >
                </TableCell>
                <TableCell class="text-xs">
                  <span v-if="row.expired" class="text-muted-foreground">{{
                    t("adminBlocks.expired")
                  }}</span>
                  <span v-else-if="row.unblocksAt">{{ fmtDate(row.unblocksAt) }}</span>
                  <span v-else class="text-muted-foreground">{{ t("adminBlocks.noExpiry") }}</span>
                </TableCell>
                <TableCell class="text-right">
                  <!-- An expired row is already unblocked; offering the verb
                       again is an action with nothing to act on. -->
                  <Button
                    v-if="!row.expired"
                    variant="outline"
                    size="sm"
                    class="text-xs h-7"
                    @click="unblockTarget = row"
                    >{{ t("adminBlocks.lift") }}</Button
                  >
                </TableCell>
              </TableRow>
            </TableBody>
          </Table>
        </CardContent>
      </Card>
    </AsyncState>

    <!-- One form, two shapes: the kind decides which fields apply. -->
    <Dialog v-model:open="dialogOpen">
      <template #title>
        {{ dialogKind === "account" ? t("adminBlocks.blockAccount") : t("adminBlocks.blockAddress") }}
      </template>
      <div class="space-y-4">
        <div class="space-y-1">
          <Label for="block-subject">{{
            dialogKind === "account" ? t("adminBlocks.userId") : t("adminBlocks.ipAddress")
          }}</Label>
          <Input id="block-subject" v-model="form.subject" class="font-mono" />
          <p v-if="selfBlockRisk" class="text-xs text-copper">
            {{ t("adminBlocks.selfLockoutWarning") }}
          </p>
        </div>
        <div class="space-y-1">
          <Label for="block-reason">{{ t("common.reason") }}</Label>
          <Input id="block-reason" v-model="form.reason" />
        </div>
        <div v-if="dialogKind === 'ip'" class="space-y-1">
          <Label for="block-duration">{{ t("adminBlocks.durationSecs") }}</Label>
          <Input id="block-duration" v-model.number="form.durationSecs" type="number" min="60" />
        </div>
        <p v-if="formError" class="text-sm text-destructive">{{ formError }}</p>
        <!-- `Dialog` has no `footer` slot — its actions live at the end of the
             default slot, as every other dialog in the console does. -->
        <div class="flex justify-end gap-2 pt-2">
          <Button variant="outline" size="sm" @click="dialogOpen = false">{{
            t("common.cancel")
          }}</Button>
          <Button
            size="sm"
            data-testid="block-submit"
            :disabled="!form.subject.trim() || formLoading"
            @click="submitBlock"
          >
            {{ formLoading ? t("adminBlocks.blocking") : t("adminBlocks.block") }}
          </Button>
        </div>
      </div>
    </Dialog>

    <!--
      `DestructiveConfirm` names IP blocks in its own doc comment as one of the
      four reasons it exists, and neither source page used it — both hand-rolled
      a plain dialog that stated consequence but never scope.
    -->
    <DestructiveConfirm
      :open="unblockTarget !== null"
      :action="t('adminBlocks.lift')"
      :count="1"
      :item-noun="
        unblockTarget?.kind === 'account' ? t('adminBlocks.account') : t('adminBlocks.ip')
      "
      :scope="unblockTarget?.subject ?? ''"
      reversible
      :loading="unblockLoading"
      :error="unblockError"
      @confirm="confirmUnblock"
      @update:open="
        (v) => {
          if (!v) {
            unblockTarget = null;
            unblockError = null;
          }
        }
      "
    />
  </div>
</template>
