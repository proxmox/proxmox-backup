Ext.define('pbs-data-store-snapshots', {
    extend: 'Ext.data.Model',
    fields: [
	'backup-type',
	'backup-id',
	{
	    name: 'backup-time',
	    type: 'date',
	    dateFormat: 'timestamp'
	},
	'files',
	'owner',
	'verification',
	{ name: 'size', type: 'int', allowNull: true, },
	{
	    name: 'crypt-mode',
	    type: 'boolean',
	    calculate: function(data) {
		let encrypted = 0;
		let crypt = {
		    none: 0,
		    mixed: 0,
		    'sign-only': 0,
		    encrypt: 0,
		    count: 0,
		};
		let signed = 0;
		data.files.forEach(file => {
		    if (file.filename === 'index.json.blob') return; // is never encrypted
		    let mode = PBS.Utils.cryptmap.indexOf(file['crypt-mode']);
		    if (mode !== -1) {
			crypt[file['crypt-mode']]++;
			crypt.count++;
		    }
		});

		return PBS.Utils.calculateCryptMode(crypt);
	    }
	},
	{
	    name: 'matchesFilter',
	    type: 'boolean',
	    defaultValue: true,
	},
    ]
});

Ext.define('PBS.DataStoreContent', {
    extend: 'Ext.tree.Panel',
    alias: 'widget.pbsDataStoreContent',

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
		'backup-group',
		'text',
		'backup-time'
	    ]);
	    Proxmox.Utils.monStoreErrors(view, this.store);
	    this.reload(); // initial load
	},

	reload: function() {
	    let view = this.getView();

	    if (!view.store || !this.store) {
		console.warn('cannot reload, no store(s)');
		return;
	    }

	    let url = `/api2/json/admin/datastore/${view.datastore}/snapshots`;
	    this.store.setProxy({
		type: 'proxmox',
		timeout: 300*1000, // 5 minutes, we should make that api call faster
		url:  url
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

		var cls = '';
		if (btype === 'vm') {
		    cls = 'fa-desktop';
		} else if (btype === 'ct') {
		    cls = 'fa-cube';
		} else if (btype === 'host') {
		    cls = 'fa-building';
		} else {
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
		    children: []
		};
	    }

	    return groups;
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
		after: () => {},
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
		    file.leaf = true;
		    file.matchesFilter = true;

		    data.children.push(file);
		}

		children.push(data);
	    }

	    let children = [];
	    for (const [name, group] of Object.entries(groups)) {
		let last_backup = 0;
		let crypt = {
		    none: 0,
		    mixed: 0,
		    'sign-only': 0,
		    encrypt: 0,
		};
		for (const item of group.children) {
		    crypt[PBS.Utils.cryptmap[item['crypt-mode']]]++;
		    if (item["backup-time"] > last_backup && item.size !== null) {
			last_backup = item["backup-time"];
			group["backup-time"] = last_backup;
			group.files = item.files;
			group.size = item.size;
			group.owner = item.owner;
			verify.lastFailed = item.verification && item.verification.state !== 'ok';
		    }
		    if (item.verification &&
		       (!group.verification || group.verification.state !== 'failed')) {
			group.verification = item.verification;
		    }

		}
		group.count = group.children.length;
		group.matchesFilter = true;
		crypt.count = group.count;
		group['crypt-mode'] = PBS.Utils.calculateCryptMode(crypt);
		group.expanded = !!expanded[name];
		children.push(group);
	    }

	    view.setRootNode({
		expanded: true,
		children: children
	    });

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
		let searchvalue = searchBox.getValue();;
		me.search(searchBox, searchvalue);
	    }
	},

	onPrune: function(view, rI, cI, item, e, rec) {
	    var view = this.getView();

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
		};
	    } else {
		params = {
		    "backup-type": data.backup_type,
		    "backup-id": data.backup_id,
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

	onForget: function(view, rI, cI, item, e, rec) {
	    let me = this;
	    var view = this.getView();

	    if (!(rec && rec.data)) return;
	    let data = rec.data;
	    if (!view.datastore) return;

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
			params: {
			    "backup-type": data["backup-type"],
			    "backup-id": data["backup-id"],
			    "backup-time": (data['backup-time'].getTime()/1000).toFixed(0),
			},
			url: `/admin/datastore/${view.datastore}/snapshots`,
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
	    let url = new URL(`/api2/json/admin/datastore/${view.datastore}/download-decoded`, window.location.origin);
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

	    Ext.create('PBS.window.FileBrowser', {
		title: `${type}/${id}/${timetext}`,
		datastore: view.datastore,
		'backup-id': id,
		'backup-time': (time.getTime()/1000).toFixed(0),
		'backup-type': type,
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
			if(me.filter(item, value)) {
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

    viewConfig: {
	getRowClass: function(record, index) {
	    let verify = record.get('verification');
	    if (verify && verify.lastFailed) {
		return 'proxmox-invalid-row';
	    }
	},
    },

    columns: [
	{
	    xtype: 'treecolumn',
	    header: gettext("Backup Group"),
	    dataIndex: 'text',
	    flex: 1
	},
	{
	    header: gettext('Actions'),
	    xtype: 'actioncolumn',
	    dataIndex: 'text',
	    items: [
		{
		    handler: 'onVerify',
		    tooltip: gettext('Verify'),
		    getClass: (v, m, rec) => rec.data.leaf ? 'pmx-hidden' : 'fa fa-search',
		    isDisabled: (v, r, c, i, rec) => !!rec.data.leaf,
		},
		{
		    handler: 'onPrune',
		    tooltip: gettext('Prune'),
		    getClass: (v, m, rec) => rec.parentNode.id ==='root' ? 'fa fa-scissors' : 'pmx-hidden',
		    isDisabled: (v, r, c, i, rec) => rec.parentNode.id !=='root',
		},
		{
		    handler: 'onForget',
		    tooltip: gettext('Forget Snapshot'),
		    getClass: (v, m, rec) => !rec.data.leaf && rec.parentNode.id !== 'root' ? 'fa critical fa-trash-o' : 'pmx-hidden',
		    isDisabled: (v, r, c, i, rec) => rec.data.leaf || rec.parentNode.id === 'root',
		},
		{
		    handler: 'downloadFile',
		    tooltip: gettext('Download'),
		    getClass: (v, m, rec) => rec.data.leaf && rec.data.filename ? 'fa fa-download' : 'pmx-hidden',
		    isDisabled: (v, r, c, i, rec) => !rec.data.leaf || !rec.data.filename || rec.data['crypt-mode'] > 2,
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
		    isDisabled: (v, r, c, i, rec) => {
			let data = rec.data;
			return !(data.leaf &&
			    data.filename &&
			    data.filename.endsWith('pxar.didx') &&
			    data['crypt-mode'] < 3);
		    }
		},
	    ]
	},
	{
	    xtype: 'datecolumn',
	    header: gettext('Backup Time'),
	    sortable: true,
	    dataIndex: 'backup-time',
	    format: 'Y-m-d H:i:s',
	    width: 150
	},
	{
	    header: gettext("Size"),
	    sortable: true,
	    dataIndex: 'size',
	    renderer: (v, meta, record) => {
		if (record.data.text === 'client.log.blob' && v === undefined) {
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
		return (iconTxt + PBS.Utils.cryptText[v]) || Proxmox.Utils.unknownText
	    }
	},
	{
	    header: gettext('Verify State'),
	    sortable: true,
	    dataIndex: 'verification',
	    renderer: (v, meta, record) => {
		if (v === undefined || v === null || !v.state) {
		    //meta.tdCls = "x-grid-row-loading";
		    return record.data.leaf ? '' : gettext('None');
		}
		let task = Proxmox.Utils.parse_task_upid(v.upid);
		let verify_time = Proxmox.Utils.render_timestamp(task.starttime);
		let iconCls = v.state === 'ok' ? 'check good' : 'times critical';
		let tip = `Verify task started on ${verify_time}`;
		if (record.parentNode.id === 'root') {
		    tip = v.state === 'ok'
			? 'All verification OK in backup group'
			: 'At least one failed verification in backup group!';
		}
		return `<span data-qtip="${tip}">
		    <i class="fa fa-fw fa-${iconCls}"></i> ${v.state}
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
	'->',
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
		}
	    },
	    listeners: {
		change: {
		    fn: 'search',
		    buffer: 500,
		},
	    },
	}
    ],
});
