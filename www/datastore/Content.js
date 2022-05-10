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

    rootVisible: true,

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
		'backup-group',
		'text',
		'backup-time',
	    ]);
	    Proxmox.Utils.monStoreErrors(view, this.store);
	},

	control: {
	    'pbsNamespaceSelector': {
		change: 'nsChange',
	    },
	},

	nsChange: function(field, value) {
	    let view = this.getView();
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
		url += `?backup-ns=${encodeURIComponent(view.namespace)}`;
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
		let { result: { data: groups } } = await Proxmox.Async.api2({
		    url: `/api2/extjs/admin/datastore/${view.datastore}/groups`,
		});
		let map = {};
		for (const group of groups) {
		    map[`${group["backup-type"]}/${group["backup-id"]}`] = group.comment;
		}
		view.getRootNode().cascade(node => {
		    if (node.parentNode && node.parentNode.id === 'root') {
			let group = `${node.data.backup_type}/${node.data.backup_id}`;
			node.set('comment', map[group], { dirty: false });
		    }
		});
	    } catch (err) {
		console.debug(err);
	    }
	},

	onLoad: function(store, records, success, operation) {
	    let me = this;
	    let view = this.getView();

	    if (!success) {
		Proxmox.Utils.setErrorMask(view, Proxmox.Utils.getResponseErrorMessage(operation.getError()));
		return;
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

		data.expanded = !!expanded[data.text];

		data.children = [];
		for (const file of data.files) {
		    file.text = file.filename;
		    file['crypt-mode'] = PBS.Utils.cryptmap.indexOf(file['crypt-mode']);
		    file.fingerprint = data.fingerprint;
		    file.leaf = true;
		    file.matchesFilter = true;

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
		children.push(group);
	    }

	    let isRootNS = !view.namespace || view.namespace === '';
	    let rootText = isRootNS
		? gettext('Root Namespace')
		: Ext.String.format(gettext("Namespace '{0}'"), view.namespace);

	    view.setRootNode({
		text: rootText,
		iconCls: "fa fa-" + (isRootNS ? 'database' : 'object-group'),
		expanded: true,
		expandable: false,
		children: children,
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

	onChangeOwner: function(view, rI, cI, item, e, rec) {
	    view = this.getView();

	    if (!rec || !rec.data || rec.parentNode.id !== 'root' || !view.datastore) {
		return;
	    }

	    let data = rec.data;

	    let win = Ext.create('PBS.BackupGroupChangeOwner', {
		datastore: view.datastore,
		backup_type: data.backup_type,
		backup_id: data.backup_id,
		owner: data.owner,
		autoShow: true,
	    });
	    win.on('destroy', this.reload, this);
	},

	onPrune: function(view, rI, cI, item, e, rec) {
	    view = this.getView();

	    if (!(rec && rec.data)) return;
	    let data = rec.data;
	    if (rec.parentNode.id !== 'root') return;

	    if (!view.datastore) return;

	    let win = Ext.create('PBS.DataStorePrune', {
		datastore: view.datastore,
		backup_type: data.backup_type,
		backup_id: data.backup_id,
	    });
	    win.on('destroy', this.reload, this);
	    win.show();
	},

	verifyAll: function() {
	    var view = this.getView();

	    Proxmox.Utils.API2Request({
		url: `/admin/datastore/${view.datastore}/verify`,
		method: 'POST',
		failure: function(response) {
		    Ext.Msg.alert(gettext('Error'), response.htmlStatus);
		},
		success: function(response, options) {
		    Ext.create('Proxmox.window.TaskViewer', {
			upid: response.result.data,
		    }).show();
		},
	    });
	},

	pruneAll: function() {
	    let me = this;
	    let view = me.getView();

	    if (!view.datastore) return;

	    Ext.create('Proxmox.window.Edit', {
		title: `Prune Datastore '${view.datastore}'`,
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
			ns: view.namespace,
			dryrun: true,
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
		listeners: {
		    destroy: () => view.down('pbsNamespaceSelector').store?.load(),
		},
	    });
	},


	onVerify: function(view, rI, cI, item, e, rec) {
	    let me = this;
	    view = me.getView();

	    if (!view.datastore) return;

	    if (!(rec && rec.data)) return;
	    let data = rec.data;

	    let params;

	    if (rec.parentNode.id !== 'root') {
		params = {
		    "backup-type": data["backup-type"],
		    "backup-id": data["backup-id"],
		    "backup-time": (data['backup-time'].getTime()/1000).toFixed(0),
		    "outdated-after": 0, // always reverify single snapshots
		};
	    } else {
		params = {
		    "backup-type": data.backup_type,
		    "backup-id": data.backup_id,
		    "outdated-after": 29, // reverify after 29 days so match with the "old" display
		};
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

	onNotesEdit: function(view, data, isGroup) {
	    let me = this;

	    let url = `/admin/datastore/${view.datastore}/`;
	    url += isGroup ? 'group-notes' : 'notes';

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

	    Ext.create('PBS.window.NotesEdit', {
		url: url,
		autoShow: true,
		apiCallDone: () => me.reload(), // FIXME: do something more efficient?
		extraRequestParams: params,
	    });
	},

	forgetGroup: function(data) {
	    let me = this;
	    let view = me.getView();

	    Ext.create('Proxmox.window.SafeDestroy', {
		url: `/admin/datastore/${view.datastore}/groups`,
		params: {
		    "backup-type": data.backup_type,
		    "backup-id": data.backup_id,
		},
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

		    Proxmox.Utils.API2Request({
			url: `/admin/datastore/${view.datastore}/snapshots`,
			params: {
			    "backup-type": data["backup-type"],
			    "backup-id": data["backup-id"],
			    "backup-time": (data['backup-time'].getTime()/1000).toFixed(0),
			},
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

	    let url = `/api2/extjs/admin/datastore/${view.datastore}/protected`;

	    Ext.create('Proxmox.window.Edit', {
		subject: gettext('Protection') + ` - ${data.text}`,
		width: 400,

		method: 'PUT',
		autoShow: true,
		isCreate: false,
		autoLoad: true,

		loadUrl: `${url}?${Ext.Object.toQueryString(params)}`,
		url,

		items: [
		    {
			xtype: 'hidden',
			name: 'backup-type',
			value: type,
		    },
		    {
			xtype: 'hidden',
			name: 'backup-id',
			value: id,
		    },
		    {
			xtype: 'hidden',
			name: 'backup-time',
			value: time,
		    },
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

	onForget: function(view, rI, cI, item, e, rec) {
	    let me = this;
	    view = this.getView();

	    if (!(rec && rec.data)) return;
	    let data = rec.data;
	    if (!view.datastore) return;

	    if (rec.parentNode.id !== 'root') {
		me.forgetSnapshot(data);
	    } else {
		me.forgetGroup(data);
	    }
	},

	downloadFile: function(tV, rI, cI, item, e, rec) {
	    let me = this;
	    let view = me.getView();

	    if (!(rec && rec.data)) return;
	    let data = rec.parentNode.data;

	    let file = rec.data.filename;
	    let params = {
		'backup-id': data['backup-id'],
		'backup-type': data['backup-type'],
		'backup-time': (data['backup-time'].getTime()/1000).toFixed(0),
		'file-name': file,
	    };

	    let idx = file.lastIndexOf('.');
	    let filename = file.slice(0, idx);
	    let atag = document.createElement('a');
	    params['file-name'] = file;
	    atag.download = filename;
	    let url = new URL(`/api2/json/admin/datastore/${view.datastore}/download-decoded`,
	                      window.location.origin);
	    for (const [key, value] of Object.entries(params)) {
		url.searchParams.append(key, value);
	    }
	    atag.href = url.href;
	    atag.click();
	},

	openPxarBrowser: function(tv, rI, Ci, item, e, rec) {
	    let me = this;
	    let view = me.getView();

	    if (!(rec && rec.data)) return;
	    let data = rec.parentNode.data;

	    let id = data['backup-id'];
	    let time = data['backup-time'];
	    let type = data['backup-type'];
	    let timetext = PBS.Utils.render_datetime_utc(data["backup-time"]);

	    Ext.create('Proxmox.window.FileBrowser', {
		title: `${type}/${id}/${timetext}`,
		listURL: `/api2/json/admin/datastore/${view.datastore}/catalog`,
		downloadURL: `/api2/json/admin/datastore/${view.datastore}/pxar-file-download`,
		enableTar: true,
		extraParams: {
		    'backup-id': id,
		    'backup-time': (time.getTime()/1000).toFixed(0),
		    'backup-type': type,
		},
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
		store.getRoot().collapseChildren(true);
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
		if (v === undefined || v === null) {
		    v = '';
		}
		v = Ext.String.htmlEncode(v);
		let icon = 'x-action-col-icon fa fa-fw fa-pencil pointer';

		return `<span class="snapshot-comment-column">${v}</span>
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
			controller.onNotesEdit(view, rec.data, rec.parentNode?.id === 'root');
		    });
		},
		dblclick: function(tree, el, row, col, ev, rec) {
		    let data = rec.data || {};
		    if (data.leaf || data.root) {
			return;
		    }
		    let view = tree.up();
		    let controller = view.controller;
		    controller.onNotesEdit(view, rec.data, rec.parentNode?.id === 'root');
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
		    getClass: (v, m, rec) => rec.data.root || rec.data.leaf ? 'pmx-hidden' : 'pve-icon-verify-lettering',
		    isActionDisabled: (v, r, c, i, rec) => !!rec.data.leaf,
                },
                {
		    handler: 'onChangeOwner',
		    getClass: (v, m, rec) => rec.parentNode && rec.parentNode.id ==='root' ? 'fa fa-user' : 'pmx-hidden',
		    getTip: (v, m, rec) => Ext.String.format(gettext("Change owner of '{0}'"), v),
		    isActionDisabled: (v, r, c, i, rec) => !rec.parentNode || rec.parentNode.id !=='root',
                },
		{
		    handler: 'onPrune',
		    getTip: (v, m, rec) => Ext.String.format(gettext("Prune '{0}'"), v),
		    getClass: (v, m, rec) => rec.parentNode && rec.parentNode.id ==='root' ? 'fa fa-scissors' : 'pmx-hidden',
		    isActionDisabled: (v, r, c, i, rec) => rec.parentNode?.id !=='root',
		},
		{
		    handler: 'onProtectionChange',
		    getTip: (v, m, rec) => Ext.String.format(gettext("Change protection of '{0}'"), v),
		    getClass: (v, m, rec) => {
			if (!rec.data.leaf && rec.parentNode && rec.parentNode.id !== 'root') {
			    let extraCls = rec.data.protected ? 'good' : 'faded';
			    return `fa fa-shield ${extraCls}`;
			}
			return 'pmx-hidden';
		    },
		    isActionDisabled: (v, r, c, i, rec) => !!rec.data.leaf || !rec.parentNode || rec.parentNode.id === 'root',
		},
		{
		    handler: 'onForget',
		    getTip: (v, m, rec) => rec.parentNode?.id !=='root'
			? Ext.String.format(gettext("Permanently forget snapshot '{0}'"), v)
			: Ext.String.format(gettext("Permanently forget group '{0}'"), v),
		    getClass: (v, m, rec) => !(rec.data.leaf || rec.data.root) ? 'fa critical fa-trash-o' : 'pmx-hidden',
		    isActionDisabled: (v, r, c, i, rec) => !!rec.data.leaf,
		},
		{
		    handler: 'downloadFile',
		    getTip: (v, m, rec) => Ext.String.format(gettext("Download '{0}'"), v),
		    getClass: (v, m, rec) => rec.data.leaf && rec.data.filename ? 'fa fa-download' : 'pmx-hidden',
		    isActionDisabled: (v, r, c, i, rec) => !rec.data.leaf || !rec.data.filename || rec.data['crypt-mode'] > 2,
		},
		{
		    handler: 'openPxarBrowser',
		    tooltip: gettext('Browse'),
		    getClass: (v, m, rec) => {
			let data = rec.data;
			if (data.leaf && data.filename && data.filename.endsWith('pxar.didx')) {
			    return 'fa fa-folder-open-o';
			}
			return 'pmx-hidden';
		    },
		    isActionDisabled: (v, r, c, i, rec) => {
			let data = rec.data;
			return !(data.leaf &&
			    data.filename &&
			    data.filename.endsWith('pxar.didx') &&
			    data['crypt-mode'] < 3);
		    },
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
	    renderer: (v, meta, record) => {
		if ((record.data.text === 'client.log.blob' && v === undefined) || record.data.root) {
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
		if (record.parentNode.id === 'root' || tip === undefined) {
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
		if (!record.parentNode) {
		    return ''; // TODO: accumulate verify of all groups into root NS node?
		}
		let i = (cls, txt) => `<i class="fa fa-fw fa-${cls}"></i> ${txt}`;
		if (v === undefined || v === null) {
		    return record.data.leaf ? '' : i('question-circle-o warning', gettext('None'));
		}
		let tip, iconCls, txt;
		if (record.parentNode.id === 'root') {
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
		    let data = rec.data || {};
		    let verify = data.verification;
		    if (verify && verify.upid && rec.parentNode.id !== 'root') {
			let win = Ext.create('Proxmox.window.TaskViewer', {
			    upid: verify.upid,
			});
			win.show();
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
	    confirmMsg: gettext('Do you want to verify all snapshots now?'),
	    handler: 'verifyAll',
	},
	{
	    xtype: 'proxmoxButton',
	    text: gettext('Prune All'),
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
