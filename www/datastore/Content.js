Ext.define('pbs-data-store-snapshots', {
    extend: 'Ext.data.Model',
    fields: [
	'backup-type',
	'backup-id',
	{
	    name: 'backup-time',
	    type: 'date',
	    dateFormat: 'timestamp',
	},
	'comment',
	'files',
	'owner',
	'verification',
	'fingerprint',
	{ name: 'size', type: 'int', allowNull: true },
	{ name: 'sortWeight', type: 'int', allowNull: true },
	{ name: 'ty', type: 'string', allowNull: true },
	{
	    name: 'crypt-mode',
	    type: 'boolean',
	    calculate: function(data) {
		let crypt = {
		    none: 0,
		    mixed: 0,
		    'sign-only': 0,
		    encrypt: 0,
		    count: 0,
		};
		data.files.forEach(file => {
		    if (file.filename === 'index.json.blob') return; // is never encrypted
		    let mode = PBS.Utils.cryptmap.indexOf(file['crypt-mode']);
		    if (mode !== -1) {
			crypt[file['crypt-mode']]++;
			crypt.count++;
		    }
		});

		return PBS.Utils.calculateCryptMode(crypt);
	    },
	},
	{
	    name: 'matchesFilter',
	    type: 'boolean',
	    defaultValue: true,
	},
    ],
});

Ext.define('PBS.DataStoreContent', {
    extend: 'Ext.tree.Panel',
    alias: 'widget.pbsDataStoreContent',
    mixins: ['Proxmox.Mixin.CBind'],

    rootVisible: false,

    title: gettext('Content'),

    controller: {
	xclass: 'Ext.app.ViewController',

	init: function(view) {
	    if (!view.datastore) {
		throw "no datastore specified";
	    }

	    this.store = Ext.create('Ext.data.Store', {
		model: 'pbs-data-store-snapshots',
		groupField: 'backup-group',
	    });
	    this.store.on('load', this.onLoad, this);

	    view.getStore().setSorters([
		'sortWeight',
		'text',
		'backup-time',
	    ]);
	},

	control: {
	    '#': { // view
		rowdblclick: 'rowDoubleClicked',
	    },
	    'pbsNamespaceSelector': {
		change: 'nsChange',
	    },
	},

	rowDoubleClicked: function(table, rec, el, rowId, ev) {
	    if (rec?.data?.ty === 'ns' && !rec.data.root) {
		this.nsChange(null, rec.data.ns);
	    }
	},

	nsChange: function(field, value) {
	    let view = this.getView();
	    if (field === null) {
		field = view.down('pbsNamespaceSelector');
		field.setValue(value);
		return;
	    }
	    view.namespace = value;
	    this.reload();
	},

	reload: function() {
	    let view = this.getView();

	    if (!view.store || !this.store) {
		console.warn('cannot reload, no store(s)');
		return;
	    }

	    let url = `/api2/json/admin/datastore/${view.datastore}/snapshots`;
	    if (view.namespace && view.namespace !== '') {
		url += `?ns=${encodeURIComponent(view.namespace)}`;
	    }
	    this.store.setProxy({
		type: 'proxmox',
		timeout: 300*1000, // 5 minutes, we should make that api call faster
		url: url,
	    });

	    this.store.load();
	},

	getRecordGroups: function(records) {
	    let groups = {};

	    for (const item of records) {
		var btype = item.data["backup-type"];
		let group = btype + "/" + item.data["backup-id"];

		if (groups[group] !== undefined) {
		    continue;
		}

		var cls = PBS.Utils.get_type_icon_cls(btype);
		if (cls === "") {
		    console.warn(`got unknown backup-type '${btype}'`);
		    continue; // FIXME: auto render? what do?
		}

		groups[group] = {
		    text: group,
		    leaf: false,
		    iconCls: "fa " + cls,
		    expanded: false,
		    backup_type: item.data["backup-type"],
		    backup_id: item.data["backup-id"],
		    children: [],
		};
	    }

	    return groups;
	},

	updateGroupNotes: async function(view) {
	    try {
		let url = `/api2/extjs/admin/datastore/${view.datastore}/groups`;
		if (view.namespace && view.namespace !== '') {
		    url += `?ns=${encodeURIComponent(view.namespace)}`;
		}
		let { result: { data: groups } } = await Proxmox.Async.api2({ url });
		let map = {};
		for (const group of groups) {
		    map[`${group["backup-type"]}/${group["backup-id"]}`] = group.comment;
		}
		view.getRootNode().cascade(node => {
		    if (node.data.ty === 'group') {
			let group = `${node.data.backup_type}/${node.data.backup_id}`;
			node.set('comment', map[group], { dirty: false });
		    }
		});
	    } catch (err) {
		console.debug(err);
	    }
	},

	loadNamespaceFromSameLevel: async function() {
	    let view = this.getView();
	    try {
		let url = `/api2/extjs/admin/datastore/${view.datastore}/namespace?max-depth=1`;
		if (view.namespace && view.namespace !== '') {
		    url += `&parent=${encodeURIComponent(view.namespace)}`;
		}
		let { result: { data: ns } } = await Proxmox.Async.api2({ url });
		return ns;
	    } catch (err) {
		console.debug(err);
	    }
	    return [];
	},

	onLoad: async function(store, records, success, operation) {
	    let me = this;
	    let view = this.getView();

	    let namespaces = await me.loadNamespaceFromSameLevel();

	    if (!success) {
		// TODO also check error code for != 403 ?
		if (namespaces.length === 0) {
		    let error = Proxmox.Utils.getResponseErrorMessage(operation.getError());
		    Proxmox.Utils.setErrorMask(view.down('treeview'), error);
		    return;
		} else {
		    records = [];
		}
	    } else {
		Proxmox.Utils.setErrorMask(view.down('treeview'));
	    }

	    let groups = this.getRecordGroups(records);

	    let selected;
	    let expanded = {};

	    view.getSelection().some(function(item) {
		let id = item.data.text;
		if (item.data.leaf) {
		    id = item.parentNode.data.text + id;
		}
		selected = id;
		return true;
	    });

	    view.getRootNode().cascadeBy({
		before: item => {
		    if (item.isExpanded() && !item.data.leaf) {
			let id = item.data.text;
			expanded[id] = true;
			return true;
		    }
		    return false;
		},
		after: Ext.emptyFn,
	    });

	    for (const item of records) {
		let group = item.data["backup-type"] + "/" + item.data["backup-id"];
		let children = groups[group].children;

		let data = item.data;

		data.text = group + '/' + PBS.Utils.render_datetime_utc(data["backup-time"]);
		data.leaf = false;
		data.cls = 'no-leaf-icons';
		data.matchesFilter = true;
		data.ty = 'dir';

		data.expanded = !!expanded[data.text];

		data.children = [];
		for (const file of data.files) {
		    file.text = file.filename;
		    file['crypt-mode'] = PBS.Utils.cryptmap.indexOf(file['crypt-mode']);
		    file.fingerprint = data.fingerprint;
		    file.leaf = true;
		    file.matchesFilter = true;
		    file.ty = 'file';

		    data.children.push(file);
		}

		children.push(data);
	    }

	    let nowSeconds = Date.now() / 1000;
	    let children = [];
	    for (const [name, group] of Object.entries(groups)) {
		let last_backup = 0;
		let crypt = {
		    none: 0,
		    mixed: 0,
		    'sign-only': 0,
		    encrypt: 0,
		};
		let verify = {
		    outdated: 0,
		    none: 0,
		    failed: 0,
		    ok: 0,
		};
		for (let item of group.children) {
		    crypt[PBS.Utils.cryptmap[item['crypt-mode']]]++;
		    if (item["backup-time"] > last_backup && item.size !== null) {
			last_backup = item["backup-time"];
			group["backup-time"] = last_backup;
			group["last-comment"] = item.comment;
			group.files = item.files;
			group.size = item.size;
			group.owner = item.owner;
			verify.lastFailed = item.verification && item.verification.state !== 'ok';
		    }
		    if (!item.verification) {
			verify.none++;
		    } else {
			if (item.verification.state === 'ok') {
			    verify.ok++;
			} else {
			    verify.failed++;
			}
			let task = Proxmox.Utils.parse_task_upid(item.verification.upid);
			item.verification.lastTime = task.starttime;
			if (nowSeconds - task.starttime > 30 * 24 * 60 * 60) {
			    verify.outdated++;
			}
		    }
		}
		group.verification = verify;
		group.count = group.children.length;
		group.matchesFilter = true;
		crypt.count = group.count;
		group['crypt-mode'] = PBS.Utils.calculateCryptMode(crypt);
		group.expanded = !!expanded[name];
		group.sortWeight = 0;
		group.ty = 'group';
		children.push(group);
	    }

	    for (const item of namespaces) {
		if (item.ns === view.namespace || (!view.namespace && item.ns === '')) {
		    continue;
		}
		children.push({
		    text: item.ns,
		    iconCls: 'fa fa-object-group',
		    expanded: true,
		    expandable: false,
		    ns: (view.namespaces ?? '') !== '' ? `/${item.ns}` : item.ns,
		    ty: 'ns',
		    sortWeight: 10,
		    leaf: true,
		});
	    }

	    let isRootNS = !view.namespace || view.namespace === '';
	    let rootText = isRootNS
		? gettext('Root Namespace')
		: Ext.String.format(gettext("Namespace '{0}'"), view.namespace);

	    let topNodes = [];
	    if (!isRootNS) {
		let parentNS = view.namespace.split('/').slice(0, -1).join('/');
		topNodes.push({
		    text: `.. (${parentNS === '' ? gettext('Root') : parentNS})`,
		    iconCls: 'fa fa-level-up',
		    ty: 'ns',
		    ns: parentNS,
		    sortWeight: -10,
		    leaf: true,
		});
	    }
	    topNodes.push({
		text: rootText,
		iconCls: "fa fa-" + (isRootNS ? 'database' : 'object-group'),
		expanded: true,
		expandable: false,
		sortWeight: -5,
		root: true, // fake root
		isRootNS,
		ty: 'ns',
		children: children,
	    });

	    view.setRootNode({
		expanded: true,
		children: topNodes,
	    });

	    if (!children.length) {
		view.setEmptyText(Ext.String.format(
		    gettext('No accessible snapshots found in namespace {0}'),
		    view.namespace && view.namespace !== '' ? `'${view.namespace}'`: gettext('Root'),
		));
	    }

	    this.updateGroupNotes(view);

	    if (selected !== undefined) {
		let selection = view.getRootNode().findChildBy(function(item) {
		    let id = item.data.text;
		    if (item.data.leaf) {
			id = item.parentNode.data.text + id;
		    }
		    return selected === id;
		}, undefined, true);
		if (selection) {
		    view.setSelection(selection);
		    view.getView().focusRow(selection);
		}
	    }

	    Proxmox.Utils.setErrorMask(view, false);
	    if (view.getStore().getFilters().length > 0) {
		let searchBox = me.lookup("searchbox");
		let searchvalue = searchBox.getValue();
		me.search(searchBox, searchvalue);
	    }
	},

	onChangeOwner: function(table, rI, cI, item, e, { data }) {
	    let view = this.getView();

	    if (data.ty !== 'group' || !view.datastore) {
		return;
	    }

	    let win = Ext.create('PBS.BackupGroupChangeOwner', {
		datastore: view.datastore,
		ns: view.namespace,
		backup_type: data.backup_type,
		backup_id: data.backup_id,
		owner: data.owner,
		autoShow: true,
	    });
	    // FIXME: don't reload all, use the record and query only its info, then update it
	    // directly in the tree
	    win.on('destroy', this.reload, this);
	},

	onPrune: function(table, rI, cI, item, e, rec) {
	    let me = this;
	    let view = me.getView();

	    if (rec.data.ty !== 'group' || !view.datastore) {
		return;
	    }
	    let data = rec.data;
	    Ext.create('PBS.DataStorePrune', {
		autoShow: true,
		datastore: view.datastore,
		ns: view.namespace,
		backup_type: data.backup_type,
		backup_id: data.backup_id,
		listeners: {
		    // FIXME: don't reload all, use the record and query only its info, then update
		    // it directly in the tree
		    destroy: () => me.reload(),
		},
	    });
	},

	verifyAll: function() {
	    let me = this;
	    let view = me.getView();

	    Ext.create('PBS.window.VerifyAll', {
		taskDone: () => me.reload(),
		autoShow: true,
		datastore: view.datastore,
		namespace: view.namespace,
	    });
	},

	pruneAll: function() {
	    let me = this;
	    let view = me.getView();

	    if (!view.datastore) return;

	    let ns = view.namespace;
	    let titleNS = ns && ns !== '' ? `Namespace '${ns}' on ` : '';

	    Ext.create('Proxmox.window.Edit', {
		title: `Prune ${titleNS}Datastore '${view.datastore}'`,
		onlineHelp: 'maintenance_pruning',
		method: 'POST',
		submitText: "Prune",
		autoShow: true,
		isCreate: true,
		showTaskViewer: true,
		taskDone: () => me.reload(),
		url: `/api2/extjs/admin/datastore/${view.datastore}/prune-datastore`,
		items: [
		    {
			xtype: 'pbsPruneInputPanel',
			ns,
			dryrun: true,
			canRecurse: true,
		    },
		],
	    });
	},

	addNS: function() {
	    let me = this;
	    let view = me.getView();
	    if (!view.datastore) return;

	    Ext.create('PBS.window.NamespaceEdit', {
		autoShow: true,
		datastore: view.datastore,
		namespace: view.namespace ?? '',
		apiCallDone: success => {
		    if (success) {
			view.down('pbsNamespaceSelector').store?.load();
			me.reload();
		    }
		},
	    });
	},

	onVerify: function(view, rI, cI, item, e, { data }) {
	    let me = this;
	    view = me.getView();

	    if ((data.ty !== 'group' && data.ty !== 'dir') || !view.datastore) {
		return;
	    }

	    let params;
	    if (data.ty === 'dir') {
		params = {
		    "backup-type": data["backup-type"],
		    "backup-id": data["backup-id"],
		    "backup-time": (data['backup-time'].getTime()/1000).toFixed(0),
		    "ignore-verified": false, // always reverify single snapshots
		};
	    } else {
		params = {
		    "backup-type": data.backup_type,
		    "backup-id": data.backup_id,
		    "outdated-after": 29, // reverify after 29 days so match with the "old" display
		};
	    }
	    if (view.namespace && view.namespace !== '') {
		params.ns = view.namespace;
	    }

	    Proxmox.Utils.API2Request({
		params: params,
		url: `/admin/datastore/${view.datastore}/verify`,
		method: 'POST',
		failure: function(response) {
		    Ext.Msg.alert(gettext('Error'), response.htmlStatus);
		},
		success: function(response, options) {
		    Ext.create('Proxmox.window.TaskViewer', {
			upid: response.result.data,
			taskDone: () => me.reload(),
		    }).show();
		},
	    });
	},

	onNotesEdit: function(view, data) {
	    let me = this;

	    let isGroup = data.ty === 'group';

	    let params;
	    if (isGroup) {
		params = {
		    "backup-type": data.backup_type,
		    "backup-id": data.backup_id,
		};
	    } else {
		params = {
		    "backup-type": data["backup-type"],
		    "backup-id": data["backup-id"],
		    "backup-time": (data['backup-time'].getTime()/1000).toFixed(0),
		};
	    }
	    if (view.namespace && view.namespace !== '') {
		params.ns = view.namespace;
	    }

	    Ext.create('PBS.window.NotesEdit', {
		url: `/admin/datastore/${view.datastore}/${isGroup ? 'group-notes' : 'notes'}`,
		autoShow: true,
		apiCallDone: () => me.reload(), // FIXME: do something more efficient?
		extraRequestParams: params,
	    });
	},

	forgetNamespace: function(data) {
	    let me = this;
	    let view = me.getView();
	    if (!view.namespace || view.namespace === '') {
		console.warn('forgetNamespace called with root NS!');
		return;
	    }
	    let nsParts = view.namespace.split('/');
	    let nsName = nsParts.pop();
	    let parentNS = nsParts.join('/');

	    Ext.create('PBS.window.NamespaceDelete', {
		datastore: view.datastore,
		namespace: view.namespace,
		item: { id: nsName },
		apiCallDone: success => {
		    if (success) {
			view.namespace = parentNS; // move up before reload to avoid "ENOENT" error
			me.reload();
		    }
		},
	    });
	},

	forgetGroup: function(data) {
	    let me = this;
	    let view = me.getView();

	    let params = {
		"backup-type": data.backup_type,
		"backup-id": data.backup_id,
	    };
	    if (view.namespace && view.namespace !== '') {
		params.ns = view.namespace;
	    }

	    Ext.create('Proxmox.window.SafeDestroy', {
		url: `/admin/datastore/${view.datastore}/groups`,
		params,
		item: {
		    id: data.text,
		},
		autoShow: true,
		taskName: 'forget-group',
		listeners: {
		    destroy: () => me.reload(),
		},
	    });
	},

	forgetSnapshot: function(data) {
	    let me = this;
	    let view = me.getView();

	    Ext.Msg.show({
		title: gettext('Confirm'),
		icon: Ext.Msg.WARNING,
		message: Ext.String.format(gettext('Are you sure you want to remove snapshot {0}'), `'${data.text}'`),
		buttons: Ext.Msg.YESNO,
		defaultFocus: 'no',
		callback: function(btn) {
		    if (btn !== 'yes') {
		        return;
		    }
		    let params = {
			"backup-type": data["backup-type"],
			"backup-id": data["backup-id"],
			"backup-time": (data['backup-time'].getTime()/1000).toFixed(0),
		    };
		    if (view.namespace && view.namespace !== '') {
			params.ns = view.namespace;
		    }

		    Proxmox.Utils.API2Request({
			url: `/admin/datastore/${view.datastore}/snapshots`,
			params,
			method: 'DELETE',
			waitMsgTarget: view,
			failure: function(response, opts) {
			    Ext.Msg.alert(gettext('Error'), response.htmlStatus);
			},
			callback: me.reload.bind(me),
		    });
		},
	    });
	},

	onProtectionChange: function(view, rI, cI, item, e, rec) {
	    let me = this;
	    view = this.getView();

	    if (!(rec && rec.data)) return;
	    let data = rec.data;
	    if (!view.datastore) return;

	    let type = data["backup-type"];
	    let id = data["backup-id"];
	    let time = (data["backup-time"].getTime()/1000).toFixed(0);

	    let params = {
		'backup-type': type,
		'backup-id': id,
		'backup-time': time,
	    };
	    if (view.namespace && view.namespace !== '') {
		params.ns = view.namespace;
	    }

	    let url = `/api2/extjs/admin/datastore/${view.datastore}/protected`;

	    Ext.create('Proxmox.window.Edit', {
		subject: gettext('Protection') + ` - ${data.text}`,
		width: 400,

		method: 'PUT',
		autoShow: true,
		isCreate: false,
		autoLoad: true,

		url,
		extraRequestParams: params,

		items: [
		    {
			xtype: 'proxmoxcheckbox',
			fieldLabel: gettext('Protected'),
			uncheckedValue: 0,
			name: 'protected',
			value: data.protected,
		    },
		],
		listeners: {
		    destroy: () => me.reload(),
		},
	    });
	},

	onForget: function(table, rI, cI, item, e, { data }) {
	    let me = this;
	    let view = this.getView();
	    if ((data.ty !== 'group' && data.ty !== 'dir' && data.ty !== 'ns') || !view.datastore) {
		return;
	    }

	    if (data.ty === 'ns') {
		me.forgetNamespace(data);
	    } else if (data.ty === 'dir') {
		me.forgetSnapshot(data);
	    } else {
		me.forgetGroup(data);
	    }
	},

	downloadFile: function(tV, rI, cI, item, e, rec) {
	    let me = this;
	    let view = me.getView();
	    if (rec.data.ty !== 'file') return;

	    let snapshot = rec.parentNode.data;
	    let file = rec.data.filename;
	    let params = {
		'backup-id': snapshot['backup-id'],
		'backup-type': snapshot['backup-type'],
		'backup-time': (snapshot['backup-time'].getTime()/1000).toFixed(0),
		'file-name': file,
	    };
	    if (view.namespace && view.namespace !== '') {
		params.ns = view.namespace;
	    }

	    let idx = file.lastIndexOf('.');
	    let filename = file.slice(0, idx);
	    let atag = document.createElement('a');
	    atag.download = filename;
	    let url = new URL(
	        `/api2/json/admin/datastore/${view.datastore}/download-decoded`,
	        window.location.origin,
	    );
	    for (const [key, value] of Object.entries(params)) {
		url.searchParams.append(key, value);
	    }
	    atag.href = url.href;
	    atag.click();
	},

	// opens either a namespace or a pxar file-browser
	openBrowser: function(tv, rI, Ci, item, e, rec) {
	    let me = this;
	    let view = me.getView();

	    if (rec.data.ty === 'ns') {
		me.nsChange(null, rec.data.ns);
		return;
	    }
	    if (rec?.data?.ty !== 'file') return;
	    let snapshot = rec.parentNode.data;

	    let id = snapshot['backup-id'];
	    let time = snapshot['backup-time'];
	    let type = snapshot['backup-type'];
	    let timetext = PBS.Utils.render_datetime_utc(snapshot["backup-time"]);
	    let extraParams = {
		'backup-id': id,
		'backup-time': (time.getTime()/1000).toFixed(0),
		'backup-type': type,
	    };
	    if (view.namespace && view.namespace !== '') {
		extraParams.ns = view.namespace;
	    }
	    Ext.create('Proxmox.window.FileBrowser', {
		title: `${type}/${id}/${timetext}`,
		listURL: `/api2/json/admin/datastore/${view.datastore}/catalog`,
		downloadURL: `/api2/json/admin/datastore/${view.datastore}/pxar-file-download`,
		extraParams,
		enableTar: true,
		downloadPrefix: `${type}-${id}-`,
		archive: rec.data.filename,
	    }).show();
	},

	filter: function(item, value) {
	    if (item.data.text.indexOf(value) !== -1) {
		return true;
	    }

	    if (item.data.owner && item.data.owner.indexOf(value) !== -1) {
		return true;
	    }

	    return false;
	},

	search: function(tf, value) {
	    let me = this;
	    let view = me.getView();
	    let store = view.getStore();
	    if (!value && value !== 0) {
		store.clearFilter();
		// only collapse the children below our toplevel namespace "root"
		store.getRoot().lastChild.collapseChildren(true);
		tf.triggers.clear.setVisible(false);
		return;
	    }
	    tf.triggers.clear.setVisible(true);
	    if (value.length < 2) return;
	    Proxmox.Utils.setErrorMask(view, true);
	    // we do it a little bit later for the error mask to work
	    setTimeout(function() {
		store.clearFilter();
		store.getRoot().collapseChildren(true);

		store.beginUpdate();
		store.getRoot().cascadeBy({
		    before: function(item) {
			if (me.filter(item, value)) {
			    item.set('matchesFilter', true);
			    if (item.parentNode && item.parentNode.id !== 'root') {
				item.parentNode.childmatches = true;
			    }
			    return false;
			}
			return true;
		    },
		    after: function(item) {
			if (me.filter(item, value) || item.id === 'root' || item.childmatches) {
			    item.set('matchesFilter', true);
			    if (item.parentNode && item.parentNode.id !== 'root') {
				item.parentNode.childmatches = true;
			    }
			    if (item.childmatches) {
				item.expand();
			    }
			} else {
			    item.set('matchesFilter', false);
			}
			delete item.childmatches;
		    },
		});
		store.endUpdate();

		store.filter((item) => !!item.get('matchesFilter'));
		Proxmox.Utils.setErrorMask(view, false);
	    }, 10);
	},
    },

    listeners: {
	activate: function() {
	    let me = this;
	    // only load on first activate to not load every tab switch
	    if (!me.firstLoad) {
		me.getController().reload();
		me.firstLoad = true;
	    }
	},
	itemcontextmenu: function(panel, record, item, index, event) {
	    event.stopEvent();
	    let menu;
	    let view = panel.up('pbsDataStoreContent');
	    let controller = view.getController();
	    let createControllerCallback = function(name) {
		return function() {
		    controller[name](view, undefined, undefined, undefined, undefined, record);
		};
	    };
	    if (record.data.ty === 'group') {
		menu = Ext.create('PBS.datastore.GroupCmdMenu', {
		    title: gettext('Group'),
		    onVerify: createControllerCallback('onVerify'),
		    onChangeOwner: createControllerCallback('onChangeOwner'),
		    onPrune: createControllerCallback('onPrune'),
		    onForget: createControllerCallback('onForget'),
		});
	    } else if (record.data.ty === 'dir') {
		menu = Ext.create('PBS.datastore.SnapshotCmdMenu', {
		    title: gettext('Snapshot'),
		    onVerify: createControllerCallback('onVerify'),
		    onProtectionChange: createControllerCallback('onProtectionChange'),
		    onForget: createControllerCallback('onForget'),
		});
	    }
	    if (menu) {
		menu.showAt(event.getXY());
	    }
	},
    },

    viewConfig: {
	getRowClass: function(record, index) {
	    let verify = record.get('verification');
	    if (verify && verify.lastFailed) {
		return 'proxmox-invalid-row';
	    }
	    return null;
	},
    },

    columns: [
	{
	    xtype: 'treecolumn',
	    header: gettext("Backup Group"),
	    dataIndex: 'text',
	    renderer: (value, meta, record) => {
		if (record.data.protected) {
		    return `${value} (${gettext('protected')})`;
		}
		return value;
	    },
	    flex: 1,
	},
	{
	    text: gettext('Comment'),
	    dataIndex: 'comment',
	    flex: 1,
	    renderer: (v, meta, record) => {
		let data = record.data;
		if (!data || data.leaf || data.root) {
		    return '';
		}

		let additionalClasses = "";
		if (!v) {
		    if (!data.expanded) {
			v = data['last-comment'] ?? '';
			additionalClasses = 'pmx-opacity-75';
		    } else {
			v = '';
		    }
		}
		v = Ext.String.htmlEncode(v);
		let icon = 'x-action-col-icon fa fa-fw fa-pencil pointer';

		return `<span class="snapshot-comment-column ${additionalClasses}">${v}</span>
		    <i data-qtip="${gettext('Edit')}" style="float: right; margin: 0px;" class="${icon}"></i>`;
	    },
	    listeners: {
		afterrender: function(component) {
		    // a bit of a hack, but relatively easy, cheap and works out well.
		    // more efficient to use one handler for the whole column than for each icon
		    component.on('click', function(tree, cell, rowI, colI, e, rec) {
			let el = e.target;
			if (el.tagName !== "I" || !el.classList.contains("fa-pencil")) {
			    return;
			}
			let view = tree.up();
			let controller = view.controller;
			controller.onNotesEdit(view, rec.data);
		    });
		},
		dblclick: function(tree, el, row, col, ev, rec) {
		    let data = rec.data || {};
		    if (data.leaf || data.root) {
			return;
		    }
		    let view = tree.up();
		    let controller = view.controller;
		    controller.onNotesEdit(view, rec.data);
		},
	    },
	},
	{
	    header: gettext('Actions'),
	    xtype: 'actioncolumn',
	    dataIndex: 'text',
	    width: 150,
	    items: [
		{
		    handler: 'onVerify',
		    getTip: (v, m, rec) => Ext.String.format(gettext("Verify '{0}'"), v),
		    getClass: (v, m, { data }) => data.ty === 'group' || data.ty === 'dir'
		        ? 'pve-icon-verify-lettering' : 'pmx-hidden',
		    isActionDisabled: (v, r, c, i, rec) => !!rec.data.leaf,
                },
                {
		    handler: 'onChangeOwner',
		    getClass: (v, m, { data }) => data.ty === 'group' ? 'fa fa-user' : 'pmx-hidden',
		    getTip: (v, m, rec) => Ext.String.format(gettext("Change owner of '{0}'"), v),
		    isActionDisabled: (v, r, c, i, { data }) => data.ty !== 'group',
                },
		{
		    handler: 'onPrune',
		    getTip: (v, m, rec) => Ext.String.format(gettext("Prune '{0}'"), v),
		    getClass: (v, m, { data }) => data.ty === 'group' ? 'fa fa-scissors' : 'pmx-hidden',
		    isActionDisabled: (v, r, c, i, { data }) => data.ty !== 'group',
		},
		{
		    handler: 'onProtectionChange',
		    getTip: (v, m, rec) => Ext.String.format(gettext("Change protection of '{0}'"), v),
		    getClass: (v, m, rec) => {
			if (rec.data.ty === 'dir') {
			    let extraCls = rec.data.protected ? 'good' : 'faded';
			    return `fa fa-shield ${extraCls}`;
			}
			return 'pmx-hidden';
		    },
		    isActionDisabled: (v, r, c, i, rec) => rec.data.ty !== 'dir',
		},
		{
		    handler: 'onForget',
		    getTip: (v, m, { data }) => {
			let tip = '{0}';
			if (data.ty === 'ns') {
			    tip = gettext("Remove namespace '{0}'");
			} else if (data.ty === 'dir') {
			    tip = gettext("Permanently forget snapshot '{0}'");
			} else if (data.ty === 'group') {
			    tip = gettext("Permanently forget group '{0}'");
			}
			return Ext.String.format(tip, v);
		    },
		    getClass: (v, m, { data }) =>
		        (data.ty === 'ns' && !data.isRootNS && data.ns === undefined) ||
		           data.ty === 'group' || data.ty === 'dir'
		        ? 'fa critical fa-trash-o'
		        : 'pmx-hidden',
		    isActionDisabled: (v, r, c, i, { data }) => false,
		},
		{
		    handler: 'downloadFile',
		    getTip: (v, m, rec) => Ext.String.format(gettext("Download '{0}'"), v),
		    getClass: (v, m, { data }) => data.ty === 'file' ? 'fa fa-download' : 'pmx-hidden',
		    isActionDisabled: (v, r, c, i, rec) => rec.data.ty !== 'file' || rec.data['crypt-mode'] > 2,
		},
		{
		    handler: 'openBrowser',
		    tooltip: gettext('Browse'),
		    getClass: (v, m, { data }) => {
			if (
			    (data.ty === 'file' && data.filename.endsWith('pxar.didx')) ||
			    (data.ty === 'ns' && !data.root)
			) {
			    return 'fa fa-folder-open-o';
			}
			return 'pmx-hidden';
		    },
		    isActionDisabled: (v, r, c, i, { data }) =>
			!(data.ty === 'file' && data.filename.endsWith('pxar.didx') && data['crypt-mode'] < 3) && data.ty !== 'ns',
		},
	    ],
	},
	{
	    xtype: 'datecolumn',
	    header: gettext('Backup Time'),
	    sortable: true,
	    dataIndex: 'backup-time',
	    format: 'Y-m-d H:i:s',
	    width: 150,
	},
	{
	    header: gettext("Size"),
	    sortable: true,
	    dataIndex: 'size',
	    renderer: (v, meta, { data }) => {
		if ((data.text === 'client.log.blob' && v === undefined) || (data.ty !== 'dir' && data.ty !== 'file')) {
		    return '';
		}
		if (v === undefined || v === null) {
		    meta.tdCls = "x-grid-row-loading";
		    return '';
		}
		return Proxmox.Utils.format_size(v);
	    },
	},
	{
	    xtype: 'numbercolumn',
	    format: '0',
	    header: gettext("Count"),
	    sortable: true,
	    width: 75,
	    align: 'right',
	    dataIndex: 'count',
	},
	{
	    header: gettext("Owner"),
	    sortable: true,
	    dataIndex: 'owner',
	},
	{
	    header: gettext('Encrypted'),
	    dataIndex: 'crypt-mode',
	    renderer: (v, meta, record) => {
		if (record.data.size === undefined || record.data.size === null) {
		    return '';
		}
		if (v === -1) {
		    return '';
		}
		let iconCls = PBS.Utils.cryptIconCls[v] || '';
		let iconTxt = "";
		if (iconCls) {
		    iconTxt = `<i class="fa fa-fw fa-${iconCls}"></i> `;
		}
		let tip;
		if (v !== PBS.Utils.cryptmap.indexOf('none') && record.data.fingerprint !== undefined) {
		    tip = "Key: " + PBS.Utils.renderKeyID(record.data.fingerprint);
		}
		let txt = (iconTxt + PBS.Utils.cryptText[v]) || Proxmox.Utils.unknownText;
		if (record.data.ty === 'group' || tip === undefined) {
		    return txt;
		} else {
		    return `<span data-qtip="${tip}">${txt}</span>`;
		}
	    },
	},
	{
	    header: gettext('Verify State'),
	    sortable: true,
	    dataIndex: 'verification',
	    width: 120,
	    sorter: (arec, brec) => {
		let a = arec.data.verification || { ok: 0, outdated: 0, failed: 0 };
		let b = brec.data.verification || { ok: 0, outdated: 0, failed: 0 };
		if (a.failed === b.failed) {
		    if (a.none === b.none) {
			if (a.outdated === b.outdated) {
			    return b.ok - a.ok;
			} else {
			    return b.outdated - a.outdated;
			}
		    } else {
			return b.none - a.none;
		    }
		} else {
		    return b.failed - a.failed;
		}
	    },
	    renderer: (v, meta, record) => {
		if (record.data.ty === 'ns') {
		    return ''; // TODO: accumulate verify of all groups into root NS node?
		}
		let i = (cls, txt) => `<i class="fa fa-fw fa-${cls}"></i> ${txt}`;
		if (v === undefined || v === null) {
		    return record.data.leaf ? '' : i('question-circle-o warning', gettext('None'));
		}
		let tip, iconCls, txt;
		if (record.data.ty === 'group') {
		    if (v.failed === 0) {
			if (v.none === 0) {
			    if (v.outdated > 0) {
				tip = 'All OK, but some snapshots were not verified in last 30 days';
				iconCls = 'check warning';
				txt = gettext('All OK (old)');
			    } else {
				tip = 'All snapshots verified at least once in last 30 days';
				iconCls = 'check good';
				txt = gettext('All OK');
			    }
			} else if (v.ok === 0) {
			    tip = `${v.none} not verified yet`;
			    iconCls = 'question-circle-o warning';
			    txt = gettext('None');
			} else {
			    tip = `${v.ok} OK, ${v.none} not verified yet`;
			    iconCls = 'check faded';
			    txt = `${v.ok} OK`;
			}
		    } else {
			tip = `${v.ok} OK, ${v.failed} failed, ${v.none} not verified yet`;
			iconCls = 'times critical';
			txt = v.ok === 0 && v.none === 0
			    ? gettext('All failed')
			    : `${v.failed} failed`;
		    }
		} else if (!v.state) {
		    return record.data.leaf ? '' : gettext('None');
		} else {
		    let verify_time = Proxmox.Utils.render_timestamp(v.lastTime);
		    tip = `Last verify task started on ${verify_time}`;
		    txt = v.state;
		    iconCls = 'times critical';
		    if (v.state === 'ok') {
			iconCls = 'check good';
			let now = Date.now() / 1000;
			if (now - v.lastTime > 30 * 24 * 60 * 60) {
			    tip = `Last verify task over 30 days ago: ${verify_time}`;
			    iconCls = 'check warning';
			}
		    }
		}
		return `<span data-qtip="${tip}">
		    <i class="fa fa-fw fa-${iconCls}"></i> ${txt}
		</span>`;
	    },
	    listeners: {
		dblclick: function(view, el, row, col, ev, rec) {
		    let verify = rec?.data?.verification;
		    if (verify?.upid && rec.parentNode?.id !== 'root') {
			Ext.create('Proxmox.window.TaskViewer', {
			    autoShow: true,
			    upid: verify.upid,
			});
		    }
		},
	    },
	},
    ],

    tbar: [
	{
	    text: gettext('Reload'),
	    iconCls: 'fa fa-refresh',
	    handler: 'reload',
	},
	'-',
	{
	    xtype: 'proxmoxButton',
	    text: gettext('Verify All'),
	    iconCls: 'pve-icon-verify-lettering',
	    handler: 'verifyAll',
	},
	{
	    xtype: 'proxmoxButton',
	    text: gettext('Prune All'),
	    iconCls: 'fa fa-scissors',
	    handler: 'pruneAll',
	},
	'->',
	{
	    xtype: 'tbtext',
	    html: gettext('Namespace') + ':',
	},
	{
	    xtype: 'pbsNamespaceSelector',
	    width: 200,
	    cbind: {
		datastore: '{datastore}',
	    },
	},
	{
	    xtype: 'proxmoxButton',
	    text: gettext('Add NS'),
	    iconCls: 'fa fa-plus-square',
	    handler: 'addNS',
	},
	'-',
	{
	    xtype: 'tbtext',
	    html: gettext('Search'),
	},
	{
	    xtype: 'textfield',
	    reference: 'searchbox',
	    emptyText: gettext('group, date or owner'),
	    triggers: {
		clear: {
		    cls: 'pmx-clear-trigger',
		    weight: -1,
		    hidden: true,
		    handler: function() {
			this.triggers.clear.setVisible(false);
			this.setValue('');
		    },
		},
	    },
	    listeners: {
		change: {
		    fn: 'search',
		    buffer: 500,
		},
	    },
	},
    ],
});

Ext.define('PBS.datastore.GroupCmdMenu', {
    extend: 'Ext.menu.Menu',
    mixins: ['Proxmox.Mixin.CBind'],

    onVerify: undefined,
    onChangeOwner: undefined,
    onPrune: undefined,
    onForget: undefined,

    items: [
	{
	    text: gettext('Verify'),
	    iconCls: 'pve-icon-verify-lettering',
	    handler: function() { this.up('menu').onVerify(); },
	    cbind: {
		hidden: '{!onVerify}',
	    },
	},
	{
	    text: gettext('Change owner'),
	    iconCls: 'fa fa-user',
	    handler: function() { this.up('menu').onChangeOwner(); },
	    cbind: {
		hidden: '{!onChangeOwner}',
	    },
	},
	{
	    text: gettext('Prune'),
	    iconCls: 'fa fa-scissors',
	    handler: function() { this.up('menu').onPrune(); },
	    cbind: {
		hidden: '{!onPrune}',
	    },
	},
	{ xtype: 'menuseparator' },
	{
	    text: gettext('Remove'),
	    iconCls: 'fa critical fa-trash-o',
	    handler: function() { this.up('menu').onForget(); },
	    cbind: {
		hidden: '{!onForget}',
	    },
	},
    ],
});

Ext.define('PBS.datastore.SnapshotCmdMenu', {
    extend: 'Ext.menu.Menu',
    mixins: ['Proxmox.Mixin.CBind'],

    onVerify: undefined,
    onProtectionChange: undefined,
    onForget: undefined,

    items: [
	{
	    text: gettext('Verify'),
	    iconCls: 'pve-icon-verify-lettering',
	    handler: function() { this.up('menu').onVerify(); },
	    cbind: {
		hidden: '{!onVerify}',
		disabled: '{!onVerify}',
	    },
	},
	{
	    text: gettext('Change Protection'),
	    iconCls: 'fa fa-shield',
	    handler: function() { this.up('menu').onProtectionChange(); },
	    cbind: {
		hidden: '{!onProtectionChange}',
		disabled: '{!onProtectionChange}',
	    },
	},
	{ xtype: 'menuseparator' },
	{
	    text: gettext('Remove'),
	    iconCls: 'fa critical fa-trash-o',
	    handler: function() { this.up('menu').onForget(); },
	    cbind: {
		hidden: '{!onForget}',
		disabled: '{!onForget}',
	    },
	},
    ],
});
