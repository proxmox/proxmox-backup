Ext.ns('PBS');

console.log("Starting Backup Server GUI");

Ext.define('PBS.Utils', {
    singleton: true,

    missingText: gettext('missing'),

    updateLoginData: function(data) {
	Proxmox.Utils.setAuthData(data);
    },

    dataStorePrefix: 'DataStore-',

    cryptmap: [
	'none',
	'mixed',
	'sign-only',
	'encrypt',
    ],

    cryptText: [
	Proxmox.Utils.noText,
	gettext('Mixed'),
	gettext('Signed'),
	gettext('Encrypted'),
    ],

    cryptIconCls: [
	'',
	'',
	'lock faded',
	'lock good',
    ],

    calculateCryptMode: function(data) {
	let mixed = data.mixed;
	let encrypted = data.encrypt;
	let signed = data['sign-only'];
	let files = data.count;
	if (mixed > 0) {
	    return PBS.Utils.cryptmap.indexOf('mixed');
	} else if (files === encrypted && encrypted > 0) {
	    return PBS.Utils.cryptmap.indexOf('encrypt');
	} else if (files === signed && signed > 0) {
	    return PBS.Utils.cryptmap.indexOf('sign-only');
	} else if ((signed+encrypted) === 0) {
	    return PBS.Utils.cryptmap.indexOf('none');
	} else {
	    return PBS.Utils.cryptmap.indexOf('mixed');
	}
    },

    noSubKeyHtml: 'You do not have a valid subscription for this server. Please visit <a target="_blank" href="https://www.proxmox.com/proxmox-backup-server/pricing">www.proxmox.com</a> to get a list of available options.',

    getDataStoreFromPath: function(path) {
	return path.slice(PBS.Utils.dataStorePrefix.length);
    },

    isDataStorePath: function(path) {
	return path.indexOf(PBS.Utils.dataStorePrefix) === 0;
    },

    parsePropertyString: function(value, defaultKey) {
	var res = {},
	    error;

	if (typeof value !== 'string' || value === '') {
	    return res;
	}

	Ext.Array.each(value.split(','), function(p) {
	    var kv = p.split('=', 2);
	    if (Ext.isDefined(kv[1])) {
		res[kv[0]] = kv[1];
	    } else if (Ext.isDefined(defaultKey)) {
		if (Ext.isDefined(res[defaultKey])) {
		    error = 'defaultKey may be only defined once in propertyString';
		    return false; // break
		}
		res[defaultKey] = kv[0];
	    } else {
		error = 'invalid propertyString, not a key=value pair and no defaultKey defined';
		return false; // break
	    }
	    return true;
	});

	if (error !== undefined) {
	    console.error(error);
	    return null;
	}

	return res;
    },

    printPropertyString: function(data, defaultKey) {
	var stringparts = [],
	    gotDefaultKeyVal = false,
	    defaultKeyVal;

	Ext.Object.each(data, function(key, value) {
	    if (defaultKey !== undefined && key === defaultKey) {
		gotDefaultKeyVal = true;
		defaultKeyVal = value;
	    } else if (value !== '' && value !== undefined) {
		stringparts.push(key + '=' + value);
	    }
	});

	stringparts = stringparts.sort();
	if (gotDefaultKeyVal) {
	    stringparts.unshift(defaultKeyVal);
	}

	return stringparts.join(',');
    },

    // helper for deleting field which are set to there default values
    delete_if_default: function(values, fieldname, default_val, create) {
	if (values[fieldname] === '' || values[fieldname] === default_val) {
	    if (!create) {
		if (values.delete) {
		    if (Ext.isArray(values.delete)) {
			values.delete.push(fieldname);
		    } else {
			values.delete += ',' + fieldname;
		    }
		} else {
		    values.delete = [fieldname];
		}
	    }

	    delete values[fieldname];
	}
    },


    render_datetime_utc: function(datetime) {
	let pad = (number) => number < 10 ? '0' + number : number;
	return datetime.getUTCFullYear() +
	    '-' + pad(datetime.getUTCMonth() + 1) +
	    '-' + pad(datetime.getUTCDate()) +
	    'T' + pad(datetime.getUTCHours()) +
	    ':' + pad(datetime.getUTCMinutes()) +
	    ':' + pad(datetime.getUTCSeconds()) +
	    'Z';
    },

    render_datastore_worker_id: function(id, what) {
	const res = id.match(/^(\S+?):(\S+?)\/(\S+?)(\/(.+))?$/);
	if (res) {
	    let datastore = res[1], backupGroup = `${res[2]}/${res[3]}`;
	    if (res[4] !== undefined) {
		let datetime = Ext.Date.parse(parseInt(res[5], 16), 'U');
		let utctime = PBS.Utils.render_datetime_utc(datetime);
		return `Datastore ${datastore} ${what} ${backupGroup}/${utctime}`;
	    } else {
		return `Datastore ${datastore} ${what} ${backupGroup}`;
	    }
	}
	return `Datastore ${what} ${id}`;
    },

    render_prune_job_worker_id: function(id, what) {
	const res = id.match(/^(\S+?):(\S+)$/);
	if (!res) {
	    return `${what} on Datastore ${id}`;
	}
	let datastore = res[1], namespace = res[2];
	return `${what} on Datastore ${datastore} Namespace  ${namespace}`;
    },

    render_tape_backup_id: function(id, what) {
	const res = id.match(/^(\S+?):(\S+?):(\S+?)(:(.+))?$/);
	if (res) {
	    let datastore = res[1];
	    let pool = res[2];
	    let drive = res[3];
	    return `${what} ${datastore} (pool ${pool}, drive ${drive})`;
	}
	return `${what} ${id}`;
    },

    render_drive_load_media_id: function(id, what) {
	const res = id.match(/^(\S+?):(\S+?)$/);
	if (res) {
	    let drive = res[1];
	    let label = res[2];
	    return gettext('Drive') + ` ${drive} - ${what} '${label}'`;
	}

	return `${what} ${id}`;
    },

    // mimics Display trait in backend
    renderKeyID: function(fingerprint) {
	return fingerprint.substring(0, 23);
    },

    render_task_status: function(value, metadata, record) {
	if (!record.data['last-run-upid']) {
	    return '-';
	}

	if (!record.data['last-run-endtime']) {
	    metadata.tdCls = 'x-grid-row-loading';
	    return '';
	}

	let parsed = Proxmox.Utils.parse_task_status(value);
	let text = value;
	let icon = '';
	switch (parsed) {
	case 'unknown':
	    icon = 'question faded';
	    text = Proxmox.Utils.unknownText;
	    break;
	case 'error':
	    icon = 'times critical';
	    text = Proxmox.Utils.errorText + ': ' + value;
	    break;
	case 'warning':
	    icon = 'exclamation warning';
	    break;
	case 'ok':
	    icon = 'check good';
	    text = gettext("OK");
	}

	return `<i class="fa fa-${icon}"></i> ${text}`;
    },

    render_next_task_run: function(value, metadat, record) {
	if (!value) return '-';

	let now = new Date();
	let next = new Date(value*1000);

	if (next < now) {
	    return gettext('pending');
	}
	return Proxmox.Utils.render_timestamp(value);
    },

    render_optional_timestamp: function(value, metadata, record) {
	if (!value) return '-';
	return Proxmox.Utils.render_timestamp(value);
    },

    parse_datastore_worker_id: function(type, id) {
	let result;
	let res;
	if (type.startsWith('verif')) {
	    res = PBS.Utils.VERIFICATION_JOB_ID_RE.exec(id);
	    if (res) {
		result = res[1];
	    }
	} else if (type.startsWith('sync')) {
	    res = PBS.Utils.SYNC_JOB_ID_RE.exec(id);
	    if (res) {
		result = res[3];
	    }
	} else if (type === 'backup') {
	    res = PBS.Utils.BACKUP_JOB_ID_RE.exec(id);
	    if (res) {
		result = res[1];
	    }
	} else if (type === 'garbage_collection') {
	    return id;
	} else if (type === 'prune') {
	    return id;
	}


	return result;
    },

    extractTokenUser: function(tokenid) {
	return tokenid.match(/^(.+)!([^!]+)$/)[1];
    },

    extractTokenName: function(tokenid) {
	return tokenid.match(/^(.+)!([^!]+)$/)[2];
    },

    render_estimate: function(value, metaData, record) {
	if (record.data.avail === 0) {
	    return gettext("Full");
	}

	if (value === undefined) {
	    return gettext('Not enough data');
	}

	let now = new Date();
	let estimate = new Date(value*1000);

	let timespan = (estimate - now)/1000;

	if (Number(estimate) <= Number(now) || isNaN(timespan)) {
	    return gettext('Never');
	}

	let duration = Proxmox.Utils.format_duration_human(timespan);
	return Ext.String.format(gettext("in {0}"), duration);
    },

    // FIXME: deprecated by Proxmox.Utils.render_size_usage ?!
    render_size_usage: function(val, max) {
	if (max === 0) {
	    return gettext('N/A');
	}
	return (val*100/max).toFixed(2) + '% (' +
	    Ext.String.format(gettext('{0} of {1}'),
	    Proxmox.Utils.format_size(val), Proxmox.Utils.format_size(max)) + ')';
    },

    get_help_tool: function(blockid) {
	let info = Proxmox.Utils.get_help_info(blockid);
	if (info === undefined) {
	    info = Proxmox.Utils.get_help_info('pbs_documentation_index');
	}
	if (info === undefined) {
	    throw "get_help_info failed"; // should not happen
	}

	let docsURI = window.location.origin + info.link;
	let title = info.title;
	if (info.subtitle) {
	    title += ' - ' + info.subtitle;
	}
	return {
	    type: 'help',
	    tooltip: title,
	    handler: function() {
		window.open(docsURI);
	    },
	};
    },

    calculate_dedup_factor: function(gcstatus) {
	let dedup = 1.0;
	if (gcstatus['disk-bytes'] > 0) {
	    dedup = (gcstatus['index-data-bytes'] || 0)/gcstatus['disk-bytes'];
	}
	return dedup;
    },

    parse_snapshot_id: function(snapshot) {
	if (!snapshot) {
	    return [undefined, undefined, undefined];
	}
	let nsRegex = /(?:^|\/)(ns\/([^/]+))/g;
	let namespaces = [];
	let nsPaths = [];
	snapshot = snapshot.replace(nsRegex, (_, nsPath, ns) => { nsPaths.push(nsPath); namespaces.push(ns); return ""; });
	let [_match, type, group, id] = /^\/?([^/]+)\/([^/]+)\/(.+)$/.exec(snapshot);

	return [type, group, id, namespaces.join('/'), nsPaths.join('/')];
    },

    get_type_icon_cls: function(btype) {
	var cls = '';
	if (btype.startsWith('vm')) {
	    cls = 'fa-desktop';
	} else if (btype.startsWith('ct')) {
	    cls = 'fa-cube';
	} else if (btype.startsWith('host')) {
	    cls = 'fa-building';
	}
	return cls;
    },

    constructor: function() {
	var me = this;

	let PROXMOX_SAFE_ID_REGEX = "([A-Za-z0-9_][A-Za-z0-9._-]*)";
	me.SAFE_ID_RE = new RegExp(`^${PROXMOX_SAFE_ID_REGEX}$`);
	// only anchored at beginning, only parses datastore for now
	me.VERIFICATION_JOB_ID_RE = new RegExp("^" + PROXMOX_SAFE_ID_REGEX + ':?');
	me.SYNC_JOB_ID_RE = new RegExp("^" + PROXMOX_SAFE_ID_REGEX + ':' +
	    PROXMOX_SAFE_ID_REGEX + ':' + PROXMOX_SAFE_ID_REGEX + ':');
	me.BACKUP_JOB_ID_RE = new RegExp("^" + PROXMOX_SAFE_ID_REGEX + ':');

	// do whatever you want here
	Proxmox.Utils.override_task_descriptions({
	    'acme-deactivate': (type, id) =>
		Ext.String.format(gettext("Deactivate {0} Account"), 'ACME') + ` '${id || 'default'}'`,
	    'acme-register': (type, id) =>
		Ext.String.format(gettext("Register {0} Account"), 'ACME') + ` '${id || 'default'}'`,
	    'acme-update': (type, id) =>
		Ext.String.format(gettext("Update {0} Account"), 'ACME') + ` '${id || 'default'}'`,
	    'acme-new-cert': ['', gettext('Order Certificate')],
	    'acme-renew-cert': ['', gettext('Renew Certificate')],
	    'acme-revoke-cert': ['', gettext('Revoke Certificate')],
	    backup: (type, id) => PBS.Utils.render_datastore_worker_id(id, gettext('Backup')),
	    'barcode-label-media': [gettext('Drive'), gettext('Barcode-Label Media')],
	    'catalog-media': [gettext('Drive'), gettext('Catalog Media')],
	    'delete-datastore': [gettext('Datastore'), gettext('Remove Datastore')],
	    'delete-namespace': [gettext('Namespace'), gettext('Remove Namespace')],
	    dircreate: [gettext('Directory Storage'), gettext('Create')],
	    dirremove: [gettext('Directory'), gettext('Remove')],
	    'eject-media': [gettext('Drive'), gettext('Eject Media')],
	    "format-media": [gettext('Drive'), gettext('Format media')],
	    "forget-group": [gettext('Group'), gettext('Remove Group')],
	    garbage_collection: ['Datastore', gettext('Garbage Collect')],
	    'realm-sync': ['Realm', gettext('User Sync')],
	    'inventory-update': [gettext('Drive'), gettext('Inventory Update')],
	    'label-media': [gettext('Drive'), gettext('Label Media')],
	    'load-media': (type, id) => PBS.Utils.render_drive_load_media_id(id, gettext('Load Media')),
	    logrotate: [null, gettext('Log Rotation')],
	    prune: (type, id) => PBS.Utils.render_datastore_worker_id(id, gettext('Prune')),
	    prunejob: (type, id) => PBS.Utils.render_prune_job_worker_id(id, gettext('Prune Job')),
	    reader: (type, id) => PBS.Utils.render_datastore_worker_id(id, gettext('Read Objects')),
	    'rewind-media': [gettext('Drive'), gettext('Rewind Media')],
	    sync: ['Datastore', gettext('Remote Sync')],
	    syncjob: [gettext('Sync Job'), gettext('Remote Sync')],
	    'tape-backup': (type, id) => PBS.Utils.render_tape_backup_id(id, gettext('Tape Backup')),
	    'tape-backup-job': (type, id) => PBS.Utils.render_tape_backup_id(id, gettext('Tape Backup Job')),
	    'tape-restore': ['Datastore', gettext('Tape Restore')],
	    'unload-media': [gettext('Drive'), gettext('Unload Media')],
	    verificationjob: [gettext('Verify Job'), gettext('Scheduled Verification')],
	    verify: ['Datastore', gettext('Verification')],
	    verify_group: ['Group', gettext('Verification')],
	    verify_snapshot: ['Snapshot', gettext('Verification')],
	    wipedisk: ['Device', gettext('Wipe Disk')],
	    zfscreate: [gettext('ZFS Storage'), gettext('Create')],
	});

	Proxmox.Schema.overrideAuthDomains({
	    pbs: {
		name: 'Proxmox Backup authentication server',
		add: false,
		edit: false,
		pwchange: true,
		sync: false,
	    },
	});
    },

    // Convert an ArrayBuffer to a base64url encoded string.
    // A `null` value will be preserved for convenience.
    bytes_to_base64url: function(bytes) {
	if (bytes === null) {
	    return null;
	}

	return btoa(Array
	    .from(new Uint8Array(bytes))
	    .map(val => String.fromCharCode(val))
	    .join(''),
	)
	.replace(/\+/g, '-')
	.replace(/\//g, '_')
	.replace(/[=]/g, '');
    },

    // Convert an a base64url string to an ArrayBuffer.
    // A `null` value will be preserved for convenience.
    base64url_to_bytes: function(b64u) {
	if (b64u === null) {
	    return null;
	}

	return new Uint8Array(
	    atob(b64u
		.replace(/-/g, '+')
		.replace(/_/g, '/'),
	    )
	    .split('')
	    .map(val => val.charCodeAt(0)),
	);
    },

    driveCommand: function(driveid, command, reqOpts) {
	let params = Ext.apply(reqOpts, {
	    url: `/api2/extjs/tape/drive/${driveid}/${command}`,
	    timeout: 5*60*1000,
	    failure: function(response) {
		Ext.Msg.alert(gettext('Error'), response.htmlStatus);
	    },
	});

	Proxmox.Utils.API2Request(params);
    },

    showMediaLabelWindow: function(response) {
	let list = [];
	for (let [key, val] of Object.entries(response.result.data)) {
	    if (key === 'ctime' || key === 'media-set-ctime') {
		val = Proxmox.Utils.render_timestamp(val);
	    }
	    list.push({ key: key, value: val });
	}

	Ext.create('Ext.window.Window', {
	    title: gettext('Label Information'),
	    modal: true,
	    width: 600,
	    height: 450,
	    layout: 'fit',
	    scrollable: true,
	    items: [
		{
		    xtype: 'grid',
		    store: {
			data: list,
		    },
		    columns: [
			{
			    text: gettext('Property'),
			    dataIndex: 'key',
			    width: 120,
			},
			{
			    text: gettext('Value'),
			    dataIndex: 'value',
			    flex: 1,
			},
		    ],
		},
	    ],
	}).show();
    },

    showCartridgeMemoryWindow: function(response) {
	Ext.create('Ext.window.Window', {
	    title: gettext('Cartridge Memory'),
	    modal: true,
	    width: 600,
	    height: 450,
	    layout: 'fit',
	    scrollable: true,
	    items: [
		{
		    xtype: 'grid',
		    store: {
			data: response.result.data,
		    },
		    columns: [
			{
			    text: gettext('ID'),
			    hidden: true,
			    dataIndex: 'id',
			    width: 60,
			},
			{
			    text: gettext('Name'),
			    dataIndex: 'name',
			    flex: 2,
			},
			{
			    text: gettext('Value'),
			    dataIndex: 'value',
			    flex: 1,
			},
		    ],
		},
	    ],
	}).show();
    },

    showVolumeStatisticsWindow: function(response) {
	let list = [];
	for (let [key, val] of Object.entries(response.result.data)) {
	    if (key === 'total-native-capacity' ||
		key === 'total-used-native-capacity' ||
		key === 'lifetime-bytes-read' ||
		key === 'lifetime-bytes-written' ||
		key === 'last-mount-bytes-read' ||
		key === 'last-mount-bytes-written') {
		val = Proxmox.Utils.format_size(val);
	    }
	    list.push({ key: key, value: val });
	}
	Ext.create('Ext.window.Window', {
	    title: gettext('Volume Statistics'),
	    modal: true,
	    width: 600,
	    height: 450,
	    layout: 'fit',
	    scrollable: true,
	    items: [
		{
		    xtype: 'grid',
		    store: {
			data: list,
		    },
		    columns: [
			{
			    text: gettext('Property'),
			    dataIndex: 'key',
			    flex: 1,
			},
			{
			    text: gettext('Value'),
			    dataIndex: 'value',
			    flex: 1,
			},
		    ],
		},
	    ],
	}).show();
    },

    showDriveStatusWindow: function(response) {
	let list = [];
	for (let [key, val] of Object.entries(response.result.data)) {
	    if (key === 'manufactured') {
		val = Proxmox.Utils.render_timestamp(val);
	    }
	    if (key === 'bytes-read' || key === 'bytes-written') {
		val = Proxmox.Utils.format_size(val);
	    }
	    list.push({ key: key, value: val });
	}

	Ext.create('Ext.window.Window', {
	    title: gettext('Status'),
	    modal: true,
	    width: 600,
	    height: 450,
	    layout: 'fit',
	    scrollable: true,
	    items: [
		{
		    xtype: 'grid',
		    store: {
			data: list,
		    },
		    columns: [
			{
			    text: gettext('Property'),
			    dataIndex: 'key',
			    width: 120,
			},
			{
			    text: gettext('Value'),
			    dataIndex: 'value',
			    flex: 1,
			},
		    ],
		},
	    ],
	}).show();
    },

    renderDriveState: function(value, md) {
	if (!value) {
	    return gettext('Idle');
	}

	let icon = '<i class="fa fa-spinner fa-pulse fa-fw"></i>';

	if (value.startsWith("UPID")) {
	    let upid = Proxmox.Utils.parse_task_upid(value);
	    md.tdCls = "pointer";
	    return `${icon} ${upid.desc}`;
	}

	return `${icon} ${value}`;
    },

    // FIXME: this "parser" is brittle and relies on the order the arguments will appear in
    parseMaintenanceMode: function(mode) {
	let [type, message] = mode.split(/,(.+)/);
	type = type.split("=").pop();
	message = message ? message.split("=")[1]
	    .replace(/^"(.*)"$/, '$1')
	    .replaceAll('\\"', '"') : null;
	return [type, message];
    },

    renderMaintenance: function(mode, activeTasks) {
	if (!mode) {
	    return gettext('None');
	}

	let [type, message] = PBS.Utils.parseMaintenanceMode(mode);

	let extra = '';

	if (activeTasks !== undefined) {
	    const conflictingTasks = activeTasks.write + (type === 'offline' ? activeTasks.read : 0);

	    if (conflictingTasks > 0) {
		extra += '| <i class="fa fa-spinner fa-pulse fa-fw"></i> ';
		extra += Ext.String.format(gettext('{0} conflicting tasks still active.'), conflictingTasks);
	    } else {
		extra += '<i class="fa fa-check"></i>';
	    }
	}

	if (message) {
	    extra += ` ("${message}")`;
	}

	let modeText = Proxmox.Utils.unknownText;
	switch (type) {
	    case 'read-only': modeText = gettext("Read-only");
		break;
	    case 'offline': modeText = gettext("Offline");
		break;
	}
	return `${modeText} ${extra}`;
    },

    render_optional_namespace: function(value, metadata, record) {
	if (!value) return `- (${gettext('Root')})`;
	return Ext.String.htmlEncode(value);
    },

    render_optional_remote: function(value, metadata, record) {
	if (!value) {
	    return `- (${gettext('Local')})`;
	}
	return Ext.String.htmlEncode(value);
    },

    tuningOptions: {
	'chunk-order': {
	    '__default__': Proxmox.Utils.defaultText + ` (${gettext('Inode')})`,
	    none: gettext('None'),
	    inode: gettext('Inode'),
	},
	'sync-level': {
	    '__default__': Proxmox.Utils.defaultText + ` (${gettext('Filesystem')})`,
	    none: gettext('None'),
	    file: gettext('File'),
	    filesystem: gettext('Filesystem'),
	},
    },

    render_tuning_options: function(tuning) {
	let options = [];
	let order = tuning['chunk-order'];
	delete tuning['chunk-order'];
	order = PBS.Utils.tuningOptions['chunk-order'][order ?? '__default__'];
	options.push(`${gettext('Chunk Order')}: ${order}`);

	let sync = tuning['sync-level'];
	delete tuning['sync-level'];
	sync = PBS.Utils.tuningOptions['sync-level'][sync ?? '__default__'];
	options.push(`${gettext('Sync Level')}: ${sync}`);

	for (const [k, v] of Object.entries(tuning)) {
	    options.push(`${k}: ${v}`);
	}

	return options.join(', ');
    },
});
